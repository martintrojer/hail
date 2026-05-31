//! Stalwart v0.16 JMAP management helpers.

use std::sync::LazyLock;
use std::time::Duration;

use rand::TryRngCore;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const JMAP_CORE: &str = "urn:ietf:params:jmap:core";
const STALWART_JMAP: &str = "urn:stalwart:jmap";
const PRINCIPAL_SET: &str = "Principal/set";
const PRINCIPAL_QUERY: &str = "Principal/query";
const PRINCIPAL_GET: &str = "Principal/get";
const QUOTA_GET: &str = "Quota/get";
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

#[derive(Clone)]
pub struct ManagementSession {
    base_url: String,
    bearer: SecretString,
    account_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagementPrincipal {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub emails: Vec<String>,
}

impl ManagementSession {
    pub async fn connect(base_url: &str, bearer: SecretString) -> Result<Self, ManagementError> {
        let account_id = management_account_id(base_url, &bearer).await?;
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            bearer,
            account_id,
        })
    }

    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    pub async fn list_domains(&self) -> Result<Vec<ManagementPrincipal>, ManagementError> {
        self.query_principals("domain", &["id", "name", "type"])
            .await
    }

    pub async fn create_domain(&self, domain: &str) -> Result<(), ManagementError> {
        let response = self
            .post_jmap(json!({
                "using": [JMAP_CORE, STALWART_JMAP],
                "methodCalls": [[
                    PRINCIPAL_SET,
                    {"accountId": self.account_id, "create": {"new-0": {"type": "domain", "name": domain}}},
                    "set-0"
                ]]
            }))
            .await?;
        ensure_set_created_or_exists(&response, PRINCIPAL_SET)
    }

    pub async fn destroy_domain(&self, domain: &str) -> Result<(), ManagementError> {
        self.destroy_principal_by_name("domain", domain).await
    }

    pub async fn list_individuals(&self) -> Result<Vec<ManagementPrincipal>, ManagementError> {
        self.query_principals(
            "individual",
            &["id", "name", "type", "description", "emails"],
        )
        .await
    }

    pub async fn create_individual(
        &self,
        email: &str,
        password: &SecretString,
        display_name: Option<&str>,
    ) -> Result<Option<String>, ManagementError> {
        let mut principal = json!({
            "type": "individual",
            "name": email,
            "secrets": [password.expose_secret()],
            "emails": [email],
            "roles": ["user"]
        });
        if let Some(name) = display_name.filter(|name| !name.is_empty()) {
            principal["description"] = Value::String(name.to_string());
        }
        let response = self
            .post_jmap(json!({
                "using": [JMAP_CORE, STALWART_JMAP],
                "methodCalls": [[
                    PRINCIPAL_SET,
                    {"accountId": self.account_id, "create": {"new-0": principal}},
                    "set-0"
                ]]
            }))
            .await?;
        ensure_set_created_or_exists(&response, PRINCIPAL_SET)?;
        Ok(created_id(&response, PRINCIPAL_SET))
    }

    pub async fn destroy_individual(&self, email: &str) -> Result<(), ManagementError> {
        self.destroy_principal_by_name("individual", email).await
    }

    pub async fn reset_individual_secret(
        &self,
        email: &str,
        password: &SecretString,
    ) -> Result<Option<String>, ManagementError> {
        let Some(principal) = self.principal_by_name("individual", email).await? else {
            return Ok(None);
        };
        let response = self
            .post_jmap(json!({
                "using": [JMAP_CORE, STALWART_JMAP],
                "methodCalls": [[
                    PRINCIPAL_SET,
                    {"accountId": self.account_id, "update": {principal.id.clone(): {"secrets": [password.expose_secret()]}}},
                    "set-0"
                ]]
            }))
            .await?;
        ensure_set_updated_or_missing(&response, PRINCIPAL_SET, &principal.id)?;
        Ok(Some(principal.id))
    }

    async fn destroy_principal_by_name(
        &self,
        principal_type: &str,
        name: &str,
    ) -> Result<(), ManagementError> {
        let Some(principal) = self.principal_by_name(principal_type, name).await? else {
            return Ok(());
        };
        let response = self
            .post_jmap(json!({
                "using": [JMAP_CORE, STALWART_JMAP],
                "methodCalls": [[
                    PRINCIPAL_SET,
                    {"accountId": self.account_id, "destroy": [principal.id.clone()]},
                    "set-0"
                ]]
            }))
            .await?;
        ensure_set_destroyed_or_missing(&response, PRINCIPAL_SET, &principal.id)
    }

    async fn principal_by_name(
        &self,
        principal_type: &str,
        name: &str,
    ) -> Result<Option<ManagementPrincipal>, ManagementError> {
        Ok(self
            .query_principals(
                principal_type,
                &["id", "name", "type", "description", "emails"],
            )
            .await?
            .into_iter()
            .find(|principal| principal.name.eq_ignore_ascii_case(name)))
    }

    async fn query_principals(
        &self,
        principal_type: &str,
        properties: &[&str],
    ) -> Result<Vec<ManagementPrincipal>, ManagementError> {
        let response = self
            .post_jmap(json!({
                "using": [JMAP_CORE, STALWART_JMAP],
                "methodCalls": [
                    [PRINCIPAL_QUERY, {"accountId": self.account_id, "filter": {"type": principal_type}}, "query-0"],
                    [PRINCIPAL_GET, {"accountId": self.account_id, "#ids": {"resultOf": "query-0", "name": PRINCIPAL_QUERY, "path": "/ids"}, "properties": properties}, "get-0"]
                ]
            }))
            .await?;
        principals_from_get(&response, PRINCIPAL_GET)
    }

    async fn post_jmap(&self, body: Value) -> Result<Value, ManagementError> {
        post_jmap(&self.base_url, &self.bearer, body).await
    }
}

pub async fn quota_used_bytes(
    jmap_url: &str,
    bearer: &SecretString,
    account_id: &str,
) -> Result<Option<u64>, ManagementError> {
    let response = post_jmap(
        jmap_url,
        bearer,
        json!({
            "using": [JMAP_CORE, STALWART_JMAP],
            "methodCalls": [[QUOTA_GET, {"accountId": account_id, "ids": null}, "quota-0"]]
        }),
    )
    .await?;
    Ok(method_response(&response, QUOTA_GET)
        .and_then(|parts| parts.get(1))
        .and_then(quota_used_from_value))
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
    let payload = set_payload(response, method)?;
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

fn ensure_set_destroyed_or_missing(
    response: &Value,
    method: &'static str,
    id: &str,
) -> Result<(), ManagementError> {
    let payload = set_payload(response, method)?;
    if payload
        .get("destroyed")
        .and_then(Value::as_array)
        .is_some_and(|destroyed| destroyed.iter().any(|item| item.as_str() == Some(id)))
    {
        return Ok(());
    }
    if let Some(error) = payload.pointer(&format!("/notDestroyed/{id}")) {
        if is_not_found(error) {
            return Ok(());
        }
        return Err(ManagementError::JmapSet {
            method,
            detail: error_detail(error),
        });
    }
    Err(ManagementError::Missing(
        "JMAP destroyed/notDestroyed result",
    ))
}

fn ensure_set_updated_or_missing(
    response: &Value,
    method: &'static str,
    id: &str,
) -> Result<(), ManagementError> {
    let payload = set_payload(response, method)?;
    if payload
        .get("updated")
        .and_then(Value::as_object)
        .is_some_and(|updated| updated.contains_key(id))
    {
        return Ok(());
    }
    if let Some(error) = payload.pointer(&format!("/notUpdated/{id}")) {
        if is_not_found(error) {
            return Ok(());
        }
        return Err(ManagementError::JmapSet {
            method,
            detail: error_detail(error),
        });
    }
    Err(ManagementError::Missing("JMAP updated/notUpdated result"))
}

fn set_payload<'a>(
    response: &'a Value,
    method: &'static str,
) -> Result<&'a Value, ManagementError> {
    let Some(parts) = method_response(response, method) else {
        if let Some(error) = method_response(response, "error") {
            return Err(ManagementError::JmapSet {
                method,
                detail: error_detail(error.get(1).unwrap_or(&Value::Null)),
            });
        }
        return Err(ManagementError::Missing("JMAP method response"));
    };
    parts.get(1).ok_or(ManagementError::Missing("JMAP payload"))
}

fn principals_from_get(
    response: &Value,
    method: &'static str,
) -> Result<Vec<ManagementPrincipal>, ManagementError> {
    let payload = method_response(response, method)
        .and_then(|parts| parts.get(1))
        .ok_or(ManagementError::Missing("JMAP Principal/get response"))?;
    let list = payload
        .get("list")
        .and_then(Value::as_array)
        .ok_or(ManagementError::Missing("JMAP Principal/get list"))?;
    list.iter().map(principal_from_value).collect()
}

fn principal_from_value(value: &Value) -> Result<ManagementPrincipal, ManagementError> {
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .ok_or(ManagementError::Missing("principal id"))?
        .to_string();
    let name = value
        .get("name")
        .or_else(|| value.get("email"))
        .and_then(Value::as_str)
        .ok_or(ManagementError::Missing("principal name"))?
        .to_string();
    let description = value
        .get("description")
        .and_then(Value::as_str)
        .map(str::to_string);
    let emails = value
        .get("emails")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    Ok(ManagementPrincipal {
        id,
        name,
        description,
        emails,
    })
}

fn method_response<'a>(response: &'a Value, method: &str) -> Option<&'a Vec<Value>> {
    response
        .get("methodResponses")?
        .as_array()?
        .iter()
        .filter_map(Value::as_array)
        .find(|parts| parts.first().and_then(Value::as_str) == Some(method))
}

fn created_id(response: &Value, method: &str) -> Option<String> {
    method_response(response, method)?
        .get(1)?
        .pointer("/created/new-0/id")?
        .as_str()
        .map(str::to_string)
}

fn quota_used_from_value(value: &Value) -> Option<u64> {
    value
        .get("list")
        .and_then(Value::as_array)
        .and_then(|items| items.iter().find_map(size_bytes_from_value))
        .or_else(|| size_bytes_from_value(value))
}

fn size_bytes_from_value(value: &Value) -> Option<u64> {
    value
        .get("used")
        .or_else(|| value.get("usedBytes"))
        .or_else(|| value.get("totalSizeBytes"))
        .or_else(|| value.get("size"))
        .or_else(|| value.get("quota").and_then(|quota| quota.get("used")))
        .and_then(Value::as_u64)
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

fn is_not_found(error: &Value) -> bool {
    let error_type = error
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if matches!(error_type, "notFound" | "not_found") {
        return true;
    }
    let lower = error_detail(error).to_ascii_lowercase();
    lower.contains("not found") || lower.contains("does not exist")
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
        let response = json!({"methodResponses": [[PRINCIPAL_SET, {"notCreated": {"new-0": {"type": "primaryKeyViolation"}}}, "0"]]});
        ensure_set_created_or_exists(&response, PRINCIPAL_SET).unwrap();
    }

    #[test]
    fn non_idempotent_error_fails() {
        let response = json!({"methodResponses": [[ACCOUNT_SET, {"notCreated": {"new-0": {"type": "invalidPatch", "description": "bad"}}}, "0"]]});
        assert!(ensure_set_created_or_exists(&response, ACCOUNT_SET).is_err());
    }

    #[test]
    fn not_found_destroy_is_idempotent() {
        let response = json!({"methodResponses": [[PRINCIPAL_SET, {"notDestroyed": {"user-id": {"type": "notFound"}}}, "0"]]});
        ensure_set_destroyed_or_missing(&response, PRINCIPAL_SET, "user-id").unwrap();
    }

    #[test]
    fn parses_principals_from_get() {
        let response = json!({"methodResponses": [[PRINCIPAL_GET, {"list": [{"id": "1", "name": "alice@example.org", "description": "Alice", "emails": ["alice@example.org"]}]}, "0"]]});
        let principals = principals_from_get(&response, PRINCIPAL_GET).unwrap();
        assert_eq!(principals[0].name, "alice@example.org");
        assert_eq!(principals[0].emails, vec!["alice@example.org"]);
    }
}
