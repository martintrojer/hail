//! Ephemeral Stalwart container fixture for integration and smoke tests.
//!
//! The fixture drives Stalwart v0.16's bootstrap and JMAP management surfaces so
//! container-backed tests start with a real domain and mailbox instead of a fake
//! or manually-provisioned account.

use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use reqwest::StatusCode;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener},
    path::Path,
    process::{Command, ExitStatus},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
use tempfile::TempDir;
use tokio::time::{Instant, sleep};

const DEFAULT_IMAGE: &str = "docker.io/stalwartlabs/stalwart:latest";
const DEFAULT_HOSTNAME: &str = "mail.hail.test";
const DEFAULT_DOMAIN: &str = "hail.test";
const DEFAULT_USER_LOCAL: &str = "alice";
const DEFAULT_USER_PASSWORD: &str = "hail-test-password";
const BOOTSTRAP_ADMIN_USER: &str = "admin";
const BOOTSTRAP_ADMIN_PASSWORD: &str = "hail-bootstrap-admin-password";
const JMAP_READY_PATH: &str = "/.well-known/jmap";
const JMAP_CORE_CAPABILITY: &str = "urn:ietf:params:jmap:core";
const STALWART_JMAP_CAPABILITY: &str = "urn:stalwart:jmap";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
const READY_TIMEOUT: Duration = Duration::from_secs(45);
const READY_POLL_INTERVAL: Duration = Duration::from_millis(500);
static NEXT_CONTAINER_ID: AtomicU64 = AtomicU64::new(1);

/// Allocate a loopback TCP port that is free at allocation time.
pub fn allocate_free_port() -> Result<u16, StalwartFixtureError> {
    let listener = TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))?;
    Ok(listener.local_addr()?.port())
}

/// Render the minimal Stalwart TOML used by the container fixture.
#[must_use]
pub fn render_stalwart_config(config: &StalwartConfig) -> String {
    format!(
        r#"[server]
hostname = "{hostname}"

[server.listener."http"]
bind = ["0.0.0.0:8080"]
protocol = "http"

[server.listener."smtp"]
bind = ["0.0.0.0:25"]
protocol = "smtp"

[storage]
data = "sqlite"
fts = "sqlite"
lookup = "sqlite"
blob = "filesystem"
directory = "internal"

[store."sqlite"]
type = "sqlite"
path = "/var/lib/stalwart/stalwart.sqlite3"

[store."sqlite".pool]
max-connections = 10
workers = 10

[store."filesystem"]
type = "fs"
path = "/var/lib/stalwart/blobs"
depth = 2

[directory."internal"]
type = "internal"
store = "sqlite"

[authentication.fallback-admin]
user = "{admin_user}"
secret = "{admin_secret}"

[tracer."stdout"]
type = "stdout"
level = "info"
ansi = false
enable = true
"#,
        hostname = config.hostname,
        admin_user = config.admin_user,
        admin_secret = config.admin_secret,
    )
}

/// Start an ephemeral Stalwart fixture with default settings.
pub async fn start_stalwart_fixture() -> Result<StalwartFixture, StalwartFixtureError> {
    StalwartFixtureBuilder::new().start().await
}

/// Builder for [`StalwartFixture`].
#[derive(Debug, Clone)]
pub struct StalwartFixtureBuilder {
    image: String,
    hostname: String,
    domain: String,
    user_local: String,
    user_password: SecretString,
    ready_timeout: Duration,
}

impl Default for StalwartFixtureBuilder {
    fn default() -> Self {
        Self {
            image: std::env::var("HAIL_STALWART_IMAGE")
                .unwrap_or_else(|_| DEFAULT_IMAGE.to_owned()),
            hostname: DEFAULT_HOSTNAME.to_owned(),
            domain: DEFAULT_DOMAIN.to_owned(),
            user_local: DEFAULT_USER_LOCAL.to_owned(),
            user_password: SecretString::from(DEFAULT_USER_PASSWORD),
            ready_timeout: READY_TIMEOUT,
        }
    }
}

impl StalwartFixtureBuilder {
    /// Create a builder with deterministic local-test defaults.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Override the container image.
    #[must_use]
    pub fn image(mut self, image: impl Into<String>) -> Self {
        self.image = image.into();
        self
    }

    /// Override Stalwart's configured hostname.
    #[must_use]
    pub fn hostname(mut self, hostname: impl Into<String>) -> Self {
        self.hostname = hostname.into();
        self
    }

    /// Override the test user's domain.
    #[must_use]
    pub fn domain(mut self, domain: impl Into<String>) -> Self {
        self.domain = domain.into();
        self
    }

    /// Override the test user's local-part.
    #[must_use]
    pub fn user_local(mut self, user_local: impl Into<String>) -> Self {
        self.user_local = user_local.into();
        self
    }

    /// Override the test user's password.
    #[must_use]
    pub fn user_password(mut self, user_password: SecretString) -> Self {
        self.user_password = user_password;
        self
    }

    /// Override how long startup waits for JMAP readiness.
    #[must_use]
    pub fn ready_timeout(mut self, timeout: Duration) -> Self {
        self.ready_timeout = timeout;
        self
    }

    /// Build config/data directories, run the container, and wait for JMAP readiness.
    pub async fn start(self) -> Result<StalwartFixture, StalwartFixtureError> {
        if !stalwart_tests_enabled() {
            return Err(StalwartFixtureError::TestsDisabled);
        }

        ensure_podman_available()?;
        let http_port = allocate_free_port()?;
        let smtp_port = allocate_free_port()?;
        let temp = TempDir::new()?;
        let etc_dir = temp.path().join("etc");
        let data_dir = temp.path().join("data");
        fs::create_dir_all(&etc_dir)?;
        fs::create_dir_all(&data_dir)?;
        make_container_writable(&etc_dir)?;
        make_container_writable(&data_dir)?;

        let unique_id = NEXT_CONTAINER_ID.fetch_add(1, Ordering::Relaxed);
        let container_name = format!("hail-stalwart-test-{}-{unique_id}", std::process::id());
        let public_url = format!("http://localhost:{http_port}");
        let recovery_admin = format!("{BOOTSTRAP_ADMIN_USER}:{BOOTSTRAP_ADMIN_PASSWORD}");
        let output = Command::new("podman")
            .args([
                "run",
                "--detach",
                "--replace",
                "--name",
                &container_name,
                "--env",
                &format!("STALWART_RECOVERY_ADMIN={recovery_admin}"),
                "--env",
                &format!("STALWART_PUBLIC_URL={public_url}"),
                "--publish",
                &format!("127.0.0.1:{http_port}:8080"),
                "--publish",
                &format!("127.0.0.1:{smtp_port}:25"),
                "--volume",
                &format!("{}:/etc/stalwart:Z", etc_dir.display()),
                "--volume",
                &format!("{}:/var/lib/stalwart:Z", data_dir.display()),
                &self.image,
            ])
            .output()?;
        if !output.status.success() {
            return Err(StalwartFixtureError::PodmanFailed {
                status: output.status,
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }

        let mut fixture = StalwartFixture {
            container_name,
            temp,
            http_port,
            smtp_port,
            image: self.image,
            hostname: self.hostname,
            domain: self.domain,
            user_local: self.user_local,
            user_password: self.user_password,
            admin_email: String::new(),
            admin_password: SecretString::from(""),
        };
        let (admin_email, admin_password) = bootstrap_stalwart(
            &fixture.jmap_url(),
            &fixture.container_name,
            &fixture.hostname,
            &fixture.domain,
            self.ready_timeout,
        )
        .await?;
        fixture.admin_email = admin_email;
        fixture.admin_password = admin_password;
        fixture.seed_default_user().await?;
        Ok(fixture)
    }
}

/// Renderable Stalwart bootstrap config inputs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StalwartConfig {
    /// Public/mail hostname used by Stalwart.
    pub hostname: String,
    /// Bootstrap fallback admin name.
    pub admin_user: String,
    /// Bootstrap fallback admin secret.
    pub admin_secret: String,
}

/// A running ephemeral Stalwart container.
#[derive(Debug)]
pub struct StalwartFixture {
    container_name: String,
    temp: TempDir,
    http_port: u16,
    smtp_port: u16,
    image: String,
    hostname: String,
    domain: String,
    user_local: String,
    user_password: SecretString,
    admin_email: String,
    admin_password: SecretString,
}

impl StalwartFixture {
    /// Name of the Podman container.
    #[must_use]
    pub fn container_name(&self) -> &str {
        &self.container_name
    }

    /// Host JMAP/HTTP port.
    #[must_use]
    pub const fn http_port(&self) -> u16 {
        self.http_port
    }

    /// Host SMTP port for future raw-message injection tests.
    #[must_use]
    pub const fn smtp_port(&self) -> u16 {
        self.smtp_port
    }

    /// Base URL for JMAP/session discovery.
    #[must_use]
    pub fn jmap_url(&self) -> String {
        format!("http://localhost:{}", self.http_port)
    }

    /// Path containing generated `/etc/stalwart/config.json` and Stalwart data.
    #[must_use]
    pub fn root_dir(&self) -> &Path {
        self.temp.path()
    }

    /// Container image used by this fixture.
    #[must_use]
    pub fn image(&self) -> &str {
        &self.image
    }

    /// Seeded test email address.
    #[must_use]
    pub fn seeded_email(&self) -> String {
        format!("{}@{}", self.user_local, self.domain)
    }

    /// Seeded user password.
    #[must_use]
    pub fn seeded_password(&self) -> &SecretString {
        &self.user_password
    }

    /// Basic-auth-as-bearer token that hail uses for Stalwart JMAP.
    #[must_use]
    pub fn seeded_basic_bearer(&self) -> SecretString {
        let token = B64.encode(format!(
            "{}:{}",
            self.seeded_email(),
            self.user_password.expose_secret()
        ));
        SecretString::from(token)
    }

    /// Obtain a JMAP session for the seeded user.
    pub async fn login_seeded_user(&self) -> Result<hail_jmap::Session, StalwartFixtureError> {
        self.login_seeded_user_via_session_url().await
    }

    async fn login_seeded_user_via_session_url(
        &self,
    ) -> Result<hail_jmap::Session, StalwartFixtureError> {
        let session = fetch_jmap_session(
            &jmap_management_client()?,
            &self.jmap_url(),
            &self.seeded_email(),
            self.user_password.expose_secret(),
        )
        .await?;
        hail_jmap::login_basic(
            &session.api_url_base(),
            &self.seeded_email(),
            self.user_password.clone(),
        )
        .await
        .map_err(StalwartFixtureError::JmapLogin)
    }

    /// Ensure the fixture's default domain and mailbox exist.
    pub async fn seed_user_domain(&self) -> Result<(), StalwartFixtureError> {
        self.create_domain(&self.domain).await?;
        self.create_user(&self.user_local, &self.user_password)
            .await
    }

    /// Ensure the fixture's default mailbox exists.
    pub async fn seed_default_user(&self) -> Result<(), StalwartFixtureError> {
        self.seed_user_domain().await
    }

    /// Create a Stalwart domain when it does not already exist.
    pub async fn create_domain(&self, domain: &str) -> Result<(), StalwartFixtureError> {
        let client = jmap_management_client()?;
        let domain_id = self.find_domain_id(&client, domain).await?;
        if domain_id.is_some() {
            return Ok(());
        }

        let request = JmapRequest::new(vec![JmapCall::new(
            "x:Domain/set",
            serde_json::json!({
                "accountId": self.admin_account_id(&client).await?,
                "create": {
                    "domain": {
                        "name": domain,
                        "certificateManagement": { "@type": "Manual" },
                        "dkimManagement": { "@type": "Manual" },
                        "dnsManagement": { "@type": "Manual" }
                    }
                }
            }),
            "0",
        )]);
        let response = post_jmap(
            &client,
            &self.jmap_url(),
            &self.admin_email,
            self.admin_password.expose_secret(),
            &request,
        )
        .await?;
        ensure_set_created(&response, "x:Domain/set", "domain")
    }

    /// Create a Stalwart user in the fixture domain when it does not already exist.
    pub async fn create_user(
        &self,
        local_part: &str,
        password: &SecretString,
    ) -> Result<(), StalwartFixtureError> {
        let client = jmap_management_client()?;
        if self.find_account_id(&client, local_part).await?.is_some() {
            return Ok(());
        }
        let Some(domain_id) = self.find_domain_id(&client, &self.domain).await? else {
            return Err(StalwartFixtureError::Provisioning(format!(
                "domain {} does not exist before user creation",
                self.domain
            )));
        };

        let request = JmapRequest::new(vec![JmapCall::new(
            "x:Account/set",
            serde_json::json!({
                "accountId": self.admin_account_id(&client).await?,
                "create": {
                    "user": {
                        "@type": "User",
                        "name": local_part,
                        "domainId": domain_id,
                        "credentials": {
                            "0": {
                                "@type": "Password",
                                "secret": password.expose_secret()
                            }
                        },
                        "roles": { "@type": "User" }
                    }
                }
            }),
            "0",
        )]);
        let response = post_jmap(
            &client,
            &self.jmap_url(),
            &self.admin_email,
            self.admin_password.expose_secret(),
            &request,
        )
        .await?;
        ensure_set_created(&response, "x:Account/set", "user")
    }

    async fn admin_account_id(
        &self,
        client: &reqwest::Client,
    ) -> Result<String, StalwartFixtureError> {
        let session = fetch_jmap_session(
            client,
            &self.jmap_url(),
            &self.admin_email,
            self.admin_password.expose_secret(),
        )
        .await?;
        session
            .primary_accounts
            .get(STALWART_JMAP_CAPABILITY)
            .or_else(|| session.primary_accounts.get("urn:ietf:params:jmap:mail"))
            .cloned()
            .ok_or_else(|| {
                StalwartFixtureError::Provisioning(
                    "admin session has no usable primary account".to_owned(),
                )
            })
    }

    async fn find_domain_id(
        &self,
        client: &reqwest::Client,
        domain: &str,
    ) -> Result<Option<String>, StalwartFixtureError> {
        let account_id = self.admin_account_id(client).await?;
        let request = JmapRequest::new(vec![
            JmapCall::new(
                "x:Domain/query",
                serde_json::json!({ "accountId": account_id }),
                "q",
            ),
            JmapCall::new(
                "x:Domain/get",
                serde_json::json!({
                    "accountId": account_id,
                    "#ids": { "resultOf": "q", "name": "x:Domain/query", "path": "/ids" },
                    "properties": ["name"]
                }),
                "g",
            ),
        ]);
        let response = post_jmap(
            client,
            &self.jmap_url(),
            &self.admin_email,
            self.admin_password.expose_secret(),
            &request,
        )
        .await?;
        find_named_object(&response, "x:Domain/get", domain)
    }

    async fn find_account_id(
        &self,
        client: &reqwest::Client,
        local_part: &str,
    ) -> Result<Option<String>, StalwartFixtureError> {
        let account_id = self.admin_account_id(client).await?;
        let request = JmapRequest::new(vec![
            JmapCall::new(
                "x:Account/query",
                serde_json::json!({ "accountId": account_id }),
                "q",
            ),
            JmapCall::new(
                "x:Account/get",
                serde_json::json!({
                    "accountId": account_id,
                    "#ids": { "resultOf": "q", "name": "x:Account/query", "path": "/ids" },
                    "properties": ["name", "domainId", "emailAddress"]
                }),
                "g",
            ),
        ]);
        let response = post_jmap(
            client,
            &self.jmap_url(),
            &self.admin_email,
            self.admin_password.expose_secret(),
            &request,
        )
        .await?;
        find_named_object(&response, "x:Account/get", local_part).map(|id| {
            id.or_else(|| {
                find_named_object(
                    &response,
                    "x:Account/get",
                    &format!("{local_part}@{}", self.domain),
                )
                .ok()
                .flatten()
            })
        })
    }
}

impl Drop for StalwartFixture {
    fn drop(&mut self) {
        let _ = Command::new("podman")
            .args(["rm", "--force", "--time", "5", &self.container_name])
            .output();
    }
}

/// Wait until Stalwart's JMAP discovery endpoint responds.
pub async fn wait_for_jmap_ready(
    base_url: &str,
    timeout: Duration,
) -> Result<(), StalwartFixtureError> {
    let client = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(StalwartFixtureError::HttpClient)?;
    let deadline = Instant::now() + timeout;
    let url = format!("{}{}", base_url.trim_end_matches('/'), JMAP_READY_PATH);
    let mut last_error: Option<String>;

    loop {
        last_error = match client.get(&url).send().await {
            Ok(response) if response.status().as_u16() < 500 => return Ok(()),
            Ok(response) => Some(format!("HTTP {}", response.status())),
            Err(err) => Some(err.to_string()),
        };

        if Instant::now() >= deadline {
            return Err(StalwartFixtureError::ReadyTimeout {
                url,
                last_error: last_error.unwrap_or_else(|| "no response".to_owned()),
            });
        }
        sleep(READY_POLL_INTERVAL).await;
    }
}

#[derive(Debug, Serialize)]
struct JmapRequest {
    using: [&'static str; 2],
    #[serde(rename = "methodCalls")]
    method_calls: Vec<JmapCall>,
}

impl JmapRequest {
    fn new(method_calls: Vec<JmapCall>) -> Self {
        Self {
            using: [JMAP_CORE_CAPABILITY, STALWART_JMAP_CAPABILITY],
            method_calls,
        }
    }
}

#[derive(Debug, Serialize)]
struct JmapCall(&'static str, serde_json::Value, &'static str);

impl JmapCall {
    fn new(method: &'static str, args: serde_json::Value, tag: &'static str) -> Self {
        Self(method, args, tag)
    }
}

#[derive(Debug, Deserialize)]
struct JmapSessionDocument {
    #[serde(rename = "primaryAccounts")]
    primary_accounts: std::collections::BTreeMap<String, String>,
    #[serde(rename = "apiUrl")]
    api_url: String,
}

impl JmapSessionDocument {
    fn api_url_base(&self) -> String {
        self.api_url
            .trim_end_matches("/jmap/")
            .trim_end_matches('/')
            .to_owned()
    }
}

#[derive(Debug, Deserialize)]
struct JmapResponse {
    #[serde(rename = "methodResponses")]
    method_responses: Vec<(String, serde_json::Value, String)>,
}

#[derive(Debug, Deserialize)]
struct BootstrapUpdated {
    username: String,
    secret: String,
}

#[derive(Debug, Deserialize)]
struct SetCreatedId {
    id: String,
}

#[derive(Debug, Deserialize)]
struct NamedObject {
    id: String,
    name: Option<String>,
    #[serde(rename = "emailAddress")]
    email_address: Option<String>,
}

async fn bootstrap_stalwart(
    base_url: &str,
    container_name: &str,
    hostname: &str,
    domain: &str,
    timeout: Duration,
) -> Result<(String, SecretString), StalwartFixtureError> {
    wait_for_jmap_ready(base_url, timeout).await?;
    let client = jmap_management_client()?;
    let mut bootstrap = fetch_bootstrap(&client, base_url).await?;
    bootstrap["serverHostname"] = serde_json::Value::String(hostname.to_owned());
    bootstrap["defaultDomain"] = serde_json::Value::String(domain.to_owned());
    bootstrap["requestTlsCertificate"] = serde_json::Value::Bool(false);
    bootstrap["generateDkimKeys"] = serde_json::Value::Bool(false);
    if let Some(object) = bootstrap.as_object_mut() {
        object.remove("id");
    }

    let request = JmapRequest::new(vec![JmapCall::new(
        "x:Bootstrap/set",
        serde_json::json!({
            "accountId": "d333333",
            "update": { "singleton": bootstrap }
        }),
        "0",
    )]);
    let response = post_jmap(
        &client,
        base_url,
        BOOTSTRAP_ADMIN_USER,
        BOOTSTRAP_ADMIN_PASSWORD,
        &request,
    )
    .await?;
    let updated = extract_bootstrap_updated(&response)?;

    restart_container(container_name)?;
    wait_for_authenticated_jmap_ready(
        &client,
        base_url,
        &updated.username,
        &updated.secret,
        timeout,
    )
    .await?;
    Ok((updated.username, SecretString::from(updated.secret)))
}

async fn fetch_bootstrap(
    client: &reqwest::Client,
    base_url: &str,
) -> Result<serde_json::Value, StalwartFixtureError> {
    let request = JmapRequest::new(vec![JmapCall::new(
        "x:Bootstrap/get",
        serde_json::json!({}),
        "0",
    )]);
    let response = post_jmap(
        client,
        base_url,
        BOOTSTRAP_ADMIN_USER,
        BOOTSTRAP_ADMIN_PASSWORD,
        &request,
    )
    .await?;
    response
        .method_responses
        .iter()
        .find(|(method, _, _)| method == "x:Bootstrap/get")
        .and_then(|(_, args, _)| args.get("list"))
        .and_then(serde_json::Value::as_array)
        .and_then(|list| list.first())
        .cloned()
        .ok_or_else(|| {
            StalwartFixtureError::Provisioning("bootstrap object was not returned".to_owned())
        })
}

async fn fetch_jmap_session(
    client: &reqwest::Client,
    base_url: &str,
    user: &str,
    password: &str,
) -> Result<JmapSessionDocument, StalwartFixtureError> {
    let url = format!("{}{}", base_url.trim_end_matches('/'), JMAP_READY_PATH);
    let response = client
        .get(url)
        .basic_auth(user, Some(password))
        .send()
        .await
        .map_err(StalwartFixtureError::Http)?;
    if response.status().is_success() {
        response.json().await.map_err(StalwartFixtureError::Http)
    } else {
        Err(StalwartFixtureError::Provisioning(format!(
            "JMAP session for {user} returned HTTP {}",
            response.status()
        )))
    }
}

async fn post_jmap(
    client: &reqwest::Client,
    base_url: &str,
    user: &str,
    password: &str,
    request: &JmapRequest,
) -> Result<JmapResponse, StalwartFixtureError> {
    let url = format!("{}/jmap/", base_url.trim_end_matches('/'));
    let response = client
        .post(url)
        .basic_auth(user, Some(password))
        .json(request)
        .send()
        .await
        .map_err(StalwartFixtureError::Http)?;
    let status = response.status();
    if status.is_success() {
        response.json().await.map_err(StalwartFixtureError::Http)
    } else {
        let body = response.text().await.unwrap_or_default();
        Err(StalwartFixtureError::ManagementHttp { status, body })
    }
}

async fn wait_for_authenticated_jmap_ready(
    client: &reqwest::Client,
    base_url: &str,
    user: &str,
    password: &str,
    timeout: Duration,
) -> Result<(), StalwartFixtureError> {
    let deadline = Instant::now() + timeout;
    let mut last_error: Option<String>;
    loop {
        match fetch_jmap_session(client, base_url, user, password).await {
            Ok(session)
                if session
                    .primary_accounts
                    .contains_key("urn:ietf:params:jmap:mail") =>
            {
                return Ok(());
            }
            Ok(_) => {
                last_error = Some("authenticated session has no mail primary account".to_owned())
            }
            Err(err) => last_error = Some(err.to_string()),
        }
        if Instant::now() >= deadline {
            return Err(StalwartFixtureError::ReadyTimeout {
                url: format!("{}{}", base_url.trim_end_matches('/'), JMAP_READY_PATH),
                last_error: last_error.unwrap_or_else(|| "no response".to_owned()),
            });
        }
        sleep(READY_POLL_INTERVAL).await;
    }
}

fn extract_bootstrap_updated(
    response: &JmapResponse,
) -> Result<BootstrapUpdated, StalwartFixtureError> {
    let args = method_args(response, "x:Bootstrap/set")?;
    if let Some(error) = args
        .get("notUpdated")
        .and_then(|value| value.get("singleton"))
    {
        return Err(StalwartFixtureError::Provisioning(format!(
            "bootstrap rejected by Stalwart: {error}"
        )));
    }
    let value = args
        .get("updated")
        .and_then(|value| value.get("singleton"))
        .cloned()
        .ok_or_else(|| {
            StalwartFixtureError::Provisioning(
                "bootstrap did not return admin credentials".to_owned(),
            )
        })?;
    serde_json::from_value(value).map_err(|err| StalwartFixtureError::Provisioning(err.to_string()))
}

fn ensure_set_created(
    response: &JmapResponse,
    method: &str,
    create_id: &str,
) -> Result<(), StalwartFixtureError> {
    let args = method_args(response, method)?;
    if let Some(created) = args
        .get("created")
        .and_then(|value| value.get(create_id))
        .cloned()
    {
        let id: SetCreatedId = serde_json::from_value(created)
            .map_err(|err| StalwartFixtureError::Provisioning(err.to_string()))?;
        if !id.id.is_empty() {
            return Ok(());
        }
    }
    if let Some(error) = args
        .get("notCreated")
        .and_then(|value| value.get(create_id))
    {
        return Err(StalwartFixtureError::Provisioning(format!(
            "{method} rejected {create_id}: {error}"
        )));
    }
    Err(StalwartFixtureError::Provisioning(format!(
        "{method} did not create {create_id}"
    )))
}

fn find_named_object(
    response: &JmapResponse,
    method: &str,
    expected_name: &str,
) -> Result<Option<String>, StalwartFixtureError> {
    let args = method_args(response, method)?;
    let Some(list) = args.get("list").and_then(serde_json::Value::as_array) else {
        return Ok(None);
    };
    for item in list {
        let object: NamedObject = serde_json::from_value(item.clone())
            .map_err(|err| StalwartFixtureError::Provisioning(err.to_string()))?;
        if object.name.as_deref() == Some(expected_name)
            || object.email_address.as_deref() == Some(expected_name)
        {
            return Ok(Some(object.id));
        }
    }
    Ok(None)
}

fn method_args<'a>(
    response: &'a JmapResponse,
    method: &str,
) -> Result<&'a serde_json::Value, StalwartFixtureError> {
    response
        .method_responses
        .iter()
        .find(|(candidate, _, _)| candidate == method)
        .map(|(_, args, _)| args)
        .ok_or_else(|| StalwartFixtureError::Provisioning(format!("missing {method} response")))
}

fn jmap_management_client() -> Result<reqwest::Client, StalwartFixtureError> {
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(StalwartFixtureError::HttpClient)
}

fn restart_container(container_name: &str) -> Result<(), StalwartFixtureError> {
    let output = Command::new("podman")
        .args(["restart", container_name])
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(StalwartFixtureError::PodmanFailed {
            status: output.status,
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

fn make_container_writable(path: &Path) -> Result<(), StalwartFixtureError> {
    let output = Command::new("podman")
        .args([
            "unshare",
            "chown",
            "-R",
            "2000:2000",
            &path.display().to_string(),
        ])
        .output();
    match output {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => Err(StalwartFixtureError::PodmanFailed {
            status: output.status,
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }),
        Err(err) => Err(StalwartFixtureError::Io(err)),
    }
}

/// True when slow/container Stalwart tests are enabled.
#[must_use]
pub fn stalwart_tests_enabled() -> bool {
    std::env::var("HAIL_RUN_STALWART_TESTS").is_ok_and(|value| value == "1")
}

fn ensure_podman_available() -> Result<(), StalwartFixtureError> {
    let output = Command::new("podman").arg("--version").output();
    match output {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => Err(StalwartFixtureError::PodmanFailed {
            status: output.status,
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            Err(StalwartFixtureError::PodmanUnavailable)
        }
        Err(err) => Err(StalwartFixtureError::Io(err)),
    }
}

/// Stalwart fixture errors.
#[derive(Debug, thiserror::Error)]
pub enum StalwartFixtureError {
    /// Container-backed tests are opt-in.
    #[error("Stalwart fixture tests are disabled; set HAIL_RUN_STALWART_TESTS=1 to run them")]
    TestsDisabled,
    /// Podman executable was not found.
    #[error("podman is not available on PATH; install podman or run inside the dev toolbox")]
    PodmanUnavailable,
    /// Podman command failed.
    #[error("podman command failed with {status}: {stderr}")]
    PodmanFailed {
        /// Exit status.
        status: ExitStatus,
        /// Captured stderr.
        stderr: String,
    },
    /// JMAP endpoint did not become ready in time.
    #[error("timed out waiting for Stalwart JMAP readiness at {url}: {last_error}")]
    ReadyTimeout {
        /// Ready URL.
        url: String,
        /// Last observed readiness error.
        last_error: String,
    },
    /// User/domain provisioning through Stalwart's management API failed.
    #[error("Stalwart user/domain provisioning failed: {0}")]
    Provisioning(String),
    /// Stalwart management endpoint returned a non-success HTTP status.
    #[error("Stalwart management HTTP request returned {status}: {body}")]
    ManagementHttp {
        /// HTTP status.
        status: StatusCode,
        /// Response body.
        body: String,
    },
    /// JMAP login for the seeded user failed.
    #[error("seeded Stalwart user could not log in through JMAP: {0}")]
    JmapLogin(hail_jmap::Error),
    /// HTTP request failed.
    #[error("Stalwart HTTP request failed: {0}")]
    Http(reqwest::Error),
    /// HTTP client setup failed.
    #[error("failed to build HTTP client: {0}")]
    HttpClient(reqwest::Error),
    /// Filesystem/network I/O failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_port_allocation_returns_valid_nonzero_port() {
        let port = allocate_free_port().expect("allocate free port");
        assert_ne!(port, 0);
    }

    #[test]
    fn rendered_config_contains_fixture_storage_and_http_listener() {
        let config = StalwartConfig {
            hostname: "mail.test.invalid".to_owned(),
            admin_user: "admin".to_owned(),
            admin_secret: "HASHED".to_owned(),
        };
        let rendered = render_stalwart_config(&config);
        assert!(rendered.contains("hostname = \"mail.test.invalid\""));
        assert!(rendered.contains("[server.listener.\"http\"]"));
        assert!(rendered.contains("bind = [\"0.0.0.0:8080\"]"));
        assert!(rendered.contains("path = \"/var/lib/stalwart/stalwart.sqlite3\""));
        assert!(rendered.contains("secret = \"HASHED\""));
    }

    #[test]
    fn seeded_bearer_uses_configured_password() {
        let fixture = StalwartFixture {
            container_name: "not-running".to_owned(),
            temp: TempDir::new().expect("tempdir"),
            http_port: 8080,
            smtp_port: 25,
            image: DEFAULT_IMAGE.to_owned(),
            hostname: DEFAULT_HOSTNAME.to_owned(),
            domain: "example.test".to_owned(),
            user_local: "me".to_owned(),
            user_password: SecretString::from("pw"),
            admin_email: "admin@example.test".to_owned(),
            admin_password: SecretString::from("admin-pw"),
        };
        assert_eq!(fixture.seeded_email(), "me@example.test");
        assert_eq!(
            fixture.seeded_basic_bearer().expose_secret(),
            "bWVAZXhhbXBsZS50ZXN0OnB3"
        );
    }
}
