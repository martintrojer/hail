//! Ephemeral Stalwart container fixture for integration and smoke tests.
//!
//! The fixture intentionally does **not** fake user/domain provisioning. It can
//! start Stalwart and prove the JMAP HTTP surface is reachable, but current hail
//! automation still needs the exact Stalwart management/auth bootstrap pinned
//! before user creation can be fully automated.

use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use secrecy::{ExposeSecret, SecretString};
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
const JMAP_READY_PATH: &str = "/.well-known/jmap";
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

        let config_path = etc_dir.join("config.toml");
        fs::write(
            &config_path,
            render_stalwart_config(&StalwartConfig {
                hostname: self.hostname,
                admin_user: "admin".to_owned(),
                admin_secret: "CHANGE_ME_WITH_OUTPUT_OF_STALWART_PWHASH".to_owned(),
            }),
        )?;

        let unique_id = NEXT_CONTAINER_ID.fetch_add(1, Ordering::Relaxed);
        let container_name = format!("hail-stalwart-test-{}-{unique_id}", std::process::id());
        let output = Command::new("podman")
            .args([
                "run",
                "--detach",
                "--replace",
                "--name",
                &container_name,
                "--publish",
                &format!("127.0.0.1:{http_port}:8080"),
                "--publish",
                &format!("127.0.0.1:{smtp_port}:25"),
                "--volume",
                &format!("{}:/opt/stalwart/etc/config.toml:ro", config_path.display()),
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

        let fixture = StalwartFixture {
            container_name,
            temp,
            http_port,
            smtp_port,
            image: self.image,
            domain: self.domain,
            user_local: self.user_local,
            user_password: self.user_password,
        };
        fixture.wait_for_jmap_ready(self.ready_timeout).await?;
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
    domain: String,
    user_local: String,
    user_password: SecretString,
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
        format!("http://127.0.0.1:{}", self.http_port)
    }

    /// Path containing generated `etc/config.toml` and Stalwart data.
    #[must_use]
    pub fn root_dir(&self) -> &Path {
        self.temp.path()
    }

    /// Container image used by this fixture.
    #[must_use]
    pub fn image(&self) -> &str {
        &self.image
    }

    /// Seeded test email address, once user provisioning is implemented.
    #[must_use]
    pub fn seeded_email(&self) -> String {
        format!("{}@{}", self.user_local, self.domain)
    }

    /// Seeded user password, once user provisioning is implemented.
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
    ///
    /// This is intentionally explicit: current Stalwart automation in hail has
    /// not yet pinned a stable management API/auth flow to create the domain and
    /// user. Once `seed_user_domain` is implemented this method should call
    /// `hail_jmap::login_bearer` and return a real session.
    pub async fn login_seeded_user(&self) -> Result<hail_jmap::Session, StalwartFixtureError> {
        let _ = self;
        Err(StalwartFixtureError::UserProvisioningNotImplemented)
    }

    /// Placeholder for creating the configured domain and user.
    pub async fn seed_user_domain(&self) -> Result<(), StalwartFixtureError> {
        let _ = self;
        Err(StalwartFixtureError::UserProvisioningNotImplemented)
    }

    async fn wait_for_jmap_ready(&self, timeout: Duration) -> Result<(), StalwartFixtureError> {
        wait_for_jmap_ready(&self.jmap_url(), timeout).await
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
    /// User/domain seeding is not automated yet.
    #[error(
        "Stalwart user/domain provisioning is not implemented; see docs/testing.md for the manual runbook"
    )]
    UserProvisioningNotImplemented,
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
    fn free_port_allocation_returns_bindable_port() {
        let port = allocate_free_port().expect("allocate free port");
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).expect("port is bindable");
        assert_eq!(listener.local_addr().expect("local addr").port(), port);
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
            domain: "example.test".to_owned(),
            user_local: "me".to_owned(),
            user_password: SecretString::from("pw"),
        };
        assert_eq!(fixture.seeded_email(), "me@example.test");
        assert_eq!(
            fixture.seeded_basic_bearer().expose_secret(),
            "bWVAZXhhbXBsZS50ZXN0OnB3"
        );
    }
}
