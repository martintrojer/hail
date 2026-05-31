//! Stalwart v0.16 JMAP management helpers.

use std::sync::LazyLock;
use std::time::Duration;

use rand::TryRngCore;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const JMAP_CORE: &str = "urn:ietf:params:jmap:core";
const STALWART_JMAP: &str = "urn:stalwart:jmap";
const DOMAIN_SET: &str = "x:Domain/set";
const DOMAIN_QUERY: &str = "x:Domain/query";
const DOMAIN_GET: &str = "x:Domain/get";
const ACCOUNT_SET: &str = "x:Account/set";
const CLIENT_ID: &str = "webadmin";
const REDIRECT_URI: &str = "https://localhost/";

static HTTP: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(30))
        .build()
        .expect("management HTTP client configuration is valid")
});

#[derive(Debug, thiserror::Error)]
pub enum ManagementError {
    #[error("failed to draw management auth nonce from OS RNG")]
    Nonce,
    #[error("stalwart management request failed: {0}")]
    Http(String),
    #[error("stalwart management API returned HTTP {status}: {detail}")]
    Api {
        status: reqwest::StatusCode,
        detail: String,
    },
    #[error("stalwart management response was missing {0}")]
    Missing(&'static str),
    #[error("stalwart management JMAP {method} failed: {detail}")]
    JmapSet {
        method: &'static str,
        detail: String,
    },
}

pub async fn login_authcode_to_bearer(
    jmap_url: &str,
    admin_user: &str,
    admin_pass: SecretString,
) -> Result<SecretString, ManagementError> {
    #[derive(Serialize)]
    struct AuthCodeRequest<'a> {
        #[serde(rename = "type")]
        kind: &'static str,
        #[serde(rename = "accountName")]
        account_name: &'a str,
        #[serde(rename = "accountSecret")]
        account_secret: &'a str,
        #[serde(rename = "clientId")]
        client_id: &'static str,
        #[serde(rename = "redirectUri")]
        redirect_uri: &'static str,
        nonce: &'a str,
    }

    let nonce = random_nonce()?;
    let auth = post_json(
        jmap_url,
        "/api/auth",
        None,
        &AuthCodeRequest {
            kind: "authCode",
            account_name: admin_user,
            account_secret: admin_pass.expose_secret(),
            client_id: CLIENT_ID,
            redirect_uri: REDIRECT_URI,
            nonce: &nonce,
        },
    )
    .await?;

    if let Some(token) = extract_access_token(&auth) {
        return Ok(SecretString::from(token));
    }
    let code = extract_client_code(&auth).ok_or(ManagementError::Missing("client_code"))?;
    exchange_client_code(jmap_url, &code).await
}

pub async fn principal_set_domain(
    jmap_url: &str,
    bearer: &SecretString,
    domain: &str,
) -> Result<(), ManagementError> {
    let account_id = management_account_id(jmap_url, bearer).await?;
    let response = post_jmap(
        jmap_url,
        bearer,
        json!({
            "using": [JMAP_CORE, STALWART_JMAP],
            "methodCalls": [[
                DOMAIN_SET,
                {"accountId": account_id, "create": {"new-0": {"name": domain, "isEnabled": true}}},
                "set-0"
            ]]
        }),
    )
    .await?;
    ensure_set_created_or_exists(&response, DOMAIN_SET)
}

pub async fn principal_set_individual(
    jmap_url: &str,
    bearer: &SecretString,
    email: &str,
    password: &SecretString,
    display_name: Option<&str>,
) -> Result<(), ManagementError> {
    let (local_part, domain) = email
        .split_once('@')
        .ok_or(ManagementError::Missing("email domain"))?;
    let account_id = management_account_id(jmap_url, bearer).await?;
    let domain_id = domain_id(jmap_url, bearer, &account_id, domain).await?;
    let mut account = json!({
        "@type": "User",
        "name": local_part,
        "domainId": domain_id,
        "credentials": {"0": {"@type": "Password", "secret": password.expose_secret()}},
        "roles": {"@type": "User"}
    });
    if let Some(name) = display_name.filter(|name| !name.is_empty()) {
        account["description"] = Value::String(name.to_string());
    }

    let response = post_jmap(
        jmap_url,
        bearer,
        json!({
            "using": [JMAP_CORE, STALWART_JMAP],
            "methodCalls": [[
                ACCOUNT_SET,
                {"accountId": account_id, "create": {"new-0": account}},
                "set-0"
            ]]
        }),
    )
    .await?;
    ensure_set_created_or_exists(&response, ACCOUNT_SET)
}

async fn exchange_client_code(jmap_url: &str, code: &str) -> Result<SecretString, ManagementError> {
    #[derive(Deserialize)]
    struct TokenResponse {
        access_token: Option<String>,
        #[serde(rename = "accessToken")]
        access_token_camel: Option<String>,
    }

    let response = HTTP
        .post(format!("{}/auth/token", jmap_url.trim_end_matches('/')))
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("client_id", CLIENT_ID),
            ("redirect_uri", REDIRECT_URI),
        ])
        .send()
        .await
        .map_err(|err| ManagementError::Http(err.to_string()))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|err| ManagementError::Http(err.to_string()))?;
    if !status.is_success() {
        return Err(ManagementError::Api {
            status,
            detail: problem_detail(&text),
        });
    }
    let token: TokenResponse = serde_json::from_str(&text)
        .map_err(|err| ManagementError::Http(format!("invalid Stalwart JSON: {err}")))?;
    token
        .access_token
        .or(token.access_token_camel)
        .map(SecretString::from)
        .ok_or(ManagementError::Missing("access_token"))
}

async fn management_account_id(
    jmap_url: &str,
    bearer: &SecretString,
) -> Result<String, ManagementError> {
    let response = HTTP
        .get(format!(
            "{}/.well-known/jmap",
            jmap_url.trim_end_matches('/')
        ))
        .bearer_auth(bearer.expose_secret())
        .send()
        .await
        .map_err(|err| ManagementError::Http(err.to_string()))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|err| ManagementError::Http(err.to_string()))?;
    if !status.is_success() {
        return Err(ManagementError::Api {
            status,
            detail: problem_detail(&text),
        });
    }
    let json: Value = serde_json::from_str(&text)
        .map_err(|err| ManagementError::Http(format!("invalid Stalwart JSON: {err}")))?;
    json.pointer("/primaryAccounts/urn:stalwart:jmap")
        .and_then(Value::as_str)
        .or_else(|| {
            json.pointer("/primaryAccounts/urn:ietf:params:jmap:mail")
                .and_then(Value::as_str)
        })
        .map(str::to_string)
        .ok_or(ManagementError::Missing("management account id"))
}

async fn domain_id(
    jmap_url: &str,
    bearer: &SecretString,
    account_id: &str,
    domain: &str,
) -> Result<String, ManagementError> {
    let response = post_jmap(
        jmap_url,
        bearer,
        json!({
            "using": [JMAP_CORE, STALWART_JMAP],
            "methodCalls": [
                [DOMAIN_QUERY, {"accountId": account_id, "filter": {"name": domain}, "limit": 2}, "0"],
                [DOMAIN_GET, {"accountId": account_id, "#ids": {"resultOf": "0", "name": DOMAIN_QUERY, "path": "/ids"}, "properties": ["name"]}, "1"]
            ]
        }),
    )
    .await?;

    response
        .get("methodResponses")
        .and_then(Value::as_array)
        .and_then(|responses| {
            responses.iter().find_map(|entry| {
                let parts = entry.as_array()?;
                if parts.first()?.as_str()? != DOMAIN_GET {
                    return None;
                }
                parts
                    .get(1)?
                    .get("list")?
                    .as_array()?
                    .iter()
                    .find(|item| item.get("name").and_then(Value::as_str) == Some(domain))?
                    .get("id")?
                    .as_str()
                    .map(str::to_string)
            })
        })
        .ok_or(ManagementError::Missing("domain id"))
}

async fn post_json<T: Serialize + ?Sized>(
    base_url: &str,
    path: &str,
    bearer: Option<&SecretString>,
    body: &T,
) -> Result<Value, ManagementError> {
    let mut request = HTTP
        .post(format!("{}{}", base_url.trim_end_matches('/'), path))
        .json(body);
    if let Some(token) = bearer {
        request = request.bearer_auth(token.expose_secret());
    }
    let response = request
        .send()
        .await
        .map_err(|err| ManagementError::Http(err.to_string()))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|err| ManagementError::Http(err.to_string()))?;
    if !status.is_success() {
        return Err(ManagementError::Api {
            status,
            detail: problem_detail(&text),
        });
    }
    if text.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&text)
        .map_err(|err| ManagementError::Http(format!("invalid Stalwart JSON: {err}")))
}

async fn post_jmap(
    base_url: &str,
    bearer: &SecretString,
    body: Value,
) -> Result<Value, ManagementError> {
    post_json(base_url, "/jmap/", Some(bearer), &body).await
}

fn ensure_set_created_or_exists(
    response: &Value,
    method: &'static str,
) -> Result<(), ManagementError> {
    let Some(parts) = method_response(response, method) else {
        if let Some(error) = method_response(response, "error") {
            return Err(ManagementError::JmapSet {
                method,
                detail: error_detail(error.get(1).unwrap_or(&Value::Null)),
            });
        }
        return Err(ManagementError::Missing("JMAP method response"));
    };
    let payload = parts
        .get(1)
        .ok_or(ManagementError::Missing("JMAP payload"))?;
    if payload
        .get("created")
        .and_then(Value::as_object)
        .is_some_and(|created| created.contains_key("new-0"))
    {
        return Ok(());
    }
    if let Some(error) = payload.pointer("/notCreated/new-0") {
        if is_already_exists(error) {
            return Ok(());
        }
        return Err(ManagementError::JmapSet {
            method,
            detail: error_detail(error),
        });
    }
    Err(ManagementError::Missing("JMAP created/notCreated result"))
}

fn method_response<'a>(response: &'a Value, method: &str) -> Option<&'a Vec<Value>> {
    response
        .get("methodResponses")?
        .as_array()?
        .iter()
        .filter_map(Value::as_array)
        .find(|parts| parts.first().and_then(Value::as_str) == Some(method))
}

fn is_already_exists(error: &Value) -> bool {
    let error_type = error
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if matches!(error_type, "alreadyExists" | "primaryKeyViolation") {
        return true;
    }
    let lower = error_detail(error).to_ascii_lowercase();
    lower.contains("already exists") || lower.contains("already exist")
}

fn error_detail(error: &Value) -> String {
    let error_type = error.get("type").and_then(Value::as_str).unwrap_or("error");
    let description = error
        .get("description")
        .or_else(|| error.get("detail"))
        .and_then(Value::as_str);
    match description {
        Some(description) if !description.is_empty() => format!("{error_type}: {description}"),
        _ => error_type.to_string(),
    }
}

fn extract_client_code(json: &Value) -> Option<String> {
    [
        "/client_code",
        "/clientCode",
        "/data/client_code",
        "/data/clientCode",
        "/data/code",
    ]
    .into_iter()
    .find_map(|pointer| json.pointer(pointer)?.as_str().map(str::to_string))
}

fn extract_access_token(json: &Value) -> Option<String> {
    [
        "/access_token",
        "/accessToken",
        "/token",
        "/data/access_token",
        "/data/accessToken",
        "/data/token",
    ]
    .into_iter()
    .find_map(|pointer| json.pointer(pointer)?.as_str().map(str::to_string))
}

fn problem_detail(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|json| {
            json.get("detail")
                .or_else(|| json.get("description"))
                .or_else(|| json.get("title"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .filter(|detail| !detail.is_empty())
        .unwrap_or_else(|| body.trim().to_string())
}

fn random_nonce() -> Result<String, ManagementError> {
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| ManagementError::Nonce)?;
    Ok(hex::encode(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_key_violation_is_idempotent() {
        let response = json!({"methodResponses": [[DOMAIN_SET, {"notCreated": {"new-0": {"type": "primaryKeyViolation"}}}, "0"]]});
        ensure_set_created_or_exists(&response, DOMAIN_SET).unwrap();
    }

    #[test]
    fn non_idempotent_error_fails() {
        let response = json!({"methodResponses": [[ACCOUNT_SET, {"notCreated": {"new-0": {"type": "invalidPatch", "description": "bad"}}}, "0"]]});
        assert!(ensure_set_created_or_exists(&response, ACCOUNT_SET).is_err());
    }
}
