use std::{
    future::Future,
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener},
    path::PathBuf,
    process::Stdio,
    time::Duration,
};

use chrono::{Duration as ChronoDuration, Utc};
use hail_jmap::jmap_client::email_submission::Property as EmailSubmissionProperty;
use hail_test::stalwart::{StalwartFixture, start_stalwart_fixture_unchecked};
use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::{process::Command, time::sleep};

const HAIL_PASSWORD: &str = "hail-test-password";
const SERVER_KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const REQUEST_HEADER: &str = "X-Hail-Request";
const DIRECT_ENV: &str = "HAIL_E2E_COMPOSE_SEND_LATER_DIRECT";
const SKIP_REASON: &str = "skipping compose send-later smoke; set HAIL_RUN_LOCAL_MAIL_TESTBED=1 to run scripts/e2e-compose-send-later-smoke.sh";

fn local_mail_testbed_enabled() -> bool {
    std::env::var("HAIL_RUN_LOCAL_MAIL_TESTBED").is_ok_and(|value| value == "1")
}

fn direct_smoke_enabled() -> bool {
    std::env::var(DIRECT_ENV).is_ok_and(|value| value == "1")
}

#[test]
fn compose_send_later_smoke_script_wrapper() {
    if !local_mail_testbed_enabled() {
        eprintln!("{SKIP_REASON}");
        return;
    }
    if direct_smoke_enabled() {
        eprintln!("skipping script wrapper inside direct script-driven cargo invocation");
        return;
    }

    let status = std::process::Command::new("scripts/e2e-compose-send-later-smoke.sh")
        .env("HAIL_RUN_LOCAL_MAIL_TESTBED", "1")
        .status()
        .expect("run compose send-later smoke script");
    assert!(status.success(), "script exited with {status}");
}

#[tokio::test]
async fn compose_send_later_smoke_flow_when_enabled() {
    if !local_mail_testbed_enabled() || !direct_smoke_enabled() {
        eprintln!("{SKIP_REASON}");
        return;
    }

    let smoke = SmokeRuntime::start()
        .await
        .expect("start compose send-later smoke runtime");
    let api = ApiClient::login(
        smoke.hail_url(),
        &smoke.stalwart.seeded_email(),
        HAIL_PASSWORD,
    )
    .await
    .expect("login to hail API");
    let user_id = api.user_id().await.expect("fetch logged-in user id");

    let send_at = Utc::now() + ChronoDuration::seconds(2);
    let response = api
        .post_json(
            "/api/compose",
            &json!({
                "to": ["bob.e2e-compose-send-later@example.net"],
                "cc": ["carol.e2e-compose-send-later@example.net"],
                "bcc": ["dana.e2e-compose-send-later@example.net"],
                "subject": format!("E2E compose send-later smoke {}", Utc::now().timestamp_nanos_opt().unwrap_or_default()),
                "body_markdown": "Scheduled smoke body with **markdown**.",
                "attachments": [],
                "send_at": send_at.to_rfc3339()
            }),
        )
        .await
        .expect("schedule compose send-later");
    assert_eq!(response["status"], "pending");
    let scheduled_send_id = response["scheduled_send_id"]
        .as_i64()
        .expect("scheduled_send_id");
    let draft_email_id = response["draft_email_id"]
        .as_str()
        .expect("draft_email_id")
        .to_owned();

    let scheduled = poll_json(
        "scheduled send status=sent",
        Duration::from_secs(45),
        || {
            let api = api.clone();
            async move {
                let json = api
                    .get_json(&format!("/api/scheduled-sends/{scheduled_send_id}"))
                    .await?;
                Ok((json["status"].as_str() == Some("sent")).then_some(json))
            }
        },
    )
    .await
    .expect("scheduled send should become sent");
    assert_eq!(scheduled["draft_email_id"], draft_email_id);
    assert!(
        scheduled["sent_at"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );
    assert!(scheduled["error"].is_null());

    assert_send_later_audit_and_event(&smoke.db, user_id, scheduled_send_id, &draft_email_id).await;
    assert_jmap_submission(&smoke.stalwart, &draft_email_id).await;
}

async fn assert_send_later_audit_and_event(
    db: &sqlx::SqlitePool,
    user_id: i64,
    scheduled_send_id: i64,
    draft_email_id: &str,
) {
    let events: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT event_type, payload_json FROM app_events WHERE user_id = ? ORDER BY id ASC",
    )
    .bind(user_id)
    .fetch_all(db)
    .await
    .expect("query app_events");
    assert!(
        events.iter().any(|(event_type, payload)| {
            event_type == "send.completed"
                && payload
                    .as_deref()
                    .and_then(|payload| serde_json::from_str::<Value>(payload).ok())
                    .is_some_and(|payload| {
                        payload["scheduled_send_id"].as_i64() == Some(scheduled_send_id)
                            && payload["draft_email_id"].as_str() == Some(draft_email_id)
                    })
        }),
        "send.completed app_event missing: {events:?}"
    );

    let audits: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT action, payload_json FROM audit_log WHERE user_id = ? ORDER BY id ASC",
    )
    .bind(user_id)
    .fetch_all(db)
    .await
    .expect("query audit log");
    for expected in ["compose.schedule", "compose.send_later.sent"] {
        assert!(
            audits.iter().any(|(action, payload)| {
                action == expected
                    && payload
                        .as_deref()
                        .and_then(|payload| serde_json::from_str::<Value>(payload).ok())
                        .is_some_and(|payload| {
                            payload["scheduled_send_id"].as_i64() == Some(scheduled_send_id)
                                && payload["draft_email_id"].as_str() == Some(draft_email_id)
                        })
            }),
            "{expected} audit row missing: {audits:?}"
        );
    }
}

async fn assert_jmap_submission(stalwart: &StalwartFixture, draft_email_id: &str) {
    let session = stalwart
        .login_seeded_user()
        .await
        .expect("seeded Stalwart user should login");
    let mut query = session
        .client()
        .email_submission_query(
            Some(
                hail_jmap::jmap_client::email_submission::query::Filter::email_ids([
                    draft_email_id,
                ]),
            ),
            None::<
                Vec<
                    hail_jmap::jmap_client::core::query::Comparator<
                        hail_jmap::jmap_client::email_submission::query::Comparator,
                    >,
                >,
            >,
        )
        .await
        .expect("EmailSubmission/query for draft email id");
    let submission_id = query
        .take_ids()
        .into_iter()
        .next()
        .expect("JMAP EmailSubmission exists for scheduled draft");
    let submission = session
        .client()
        .email_submission_get(
            &submission_id,
            Some(vec![
                EmailSubmissionProperty::Id,
                EmailSubmissionProperty::EmailId,
                EmailSubmissionProperty::Envelope,
            ]),
        )
        .await
        .expect("EmailSubmission/get should succeed")
        .expect("submission should exist");
    assert_eq!(submission.email_id(), Some(draft_email_id));
    assert_eq!(
        submission.mail_from().map(|addr| addr.email()),
        Some("alice@hail.test")
    );
    let recipients = submission
        .rcpt_to()
        .expect("submission envelope rcptTo")
        .iter()
        .map(|addr| addr.email().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        recipients,
        vec![
            "bob.e2e-compose-send-later@example.net",
            "carol.e2e-compose-send-later@example.net",
            "dana.e2e-compose-send-later@example.net",
        ]
    );
}

struct SmokeRuntime {
    _temp: TempDir,
    stalwart: StalwartFixture,
    api: ChildProcess,
    worker: ChildProcess,
    db: sqlx::SqlitePool,
    hail_url: String,
}

impl SmokeRuntime {
    async fn start() -> Result<Self, Box<dyn std::error::Error>> {
        let temp = TempDir::new()?;
        let db_path = temp.path().join("hail.db");
        let api_port = allocate_free_port()?;
        let hail_url = format!("http://127.0.0.1:{api_port}");
        let stalwart = start_stalwart_fixture_unchecked().await?;
        let jmap_url = stalwart.jmap_url();
        let db_url = format!("sqlite://{}", db_path.display());
        let db = hail_db::connect(&db_url).await?;
        let target_dir = target_dir()?;

        let mut api = Command::new(target_dir.join("hail-api"));
        configure_process(&mut api, &db_url, &jmap_url, &hail_url, SERVER_KEY)
            .env("HAIL_SERVER__BIND", format!("127.0.0.1:{api_port}"));
        let api = ChildProcess::spawn("hail-api", api)?;
        wait_for_ready(&hail_url).await?;

        let mut worker = Command::new(target_dir.join("hail-worker"));
        configure_process(&mut worker, &db_url, &jmap_url, &hail_url, SERVER_KEY)
            .env("HAIL_TICK_SECS", "1")
            .env("HAIL_RECONCILE_EVERY_SECS", "3600");
        let worker = ChildProcess::spawn("hail-worker", worker)?;

        Ok(Self {
            _temp: temp,
            stalwart,
            api,
            worker,
            db,
            hail_url,
        })
    }

    fn hail_url(&self) -> &str {
        &self.hail_url
    }
}

impl Drop for SmokeRuntime {
    fn drop(&mut self) {
        self.api.terminate();
        self.worker.terminate();
    }
}

fn configure_process<'a>(
    command: &'a mut Command,
    db_url: &str,
    jmap_url: &str,
    hail_url: &str,
    server_key: &str,
) -> &'a mut Command {
    command
        .env("HAIL_DATABASE_URL", db_url)
        .env("HAIL_STALWART__JMAP_URL", jmap_url)
        .env("HAIL_SERVER__BIND", "127.0.0.1:0")
        .env("HAIL_SERVER__PUBLIC_URL", hail_url)
        .env("HAIL_SECRETS__SERVER_KEY", server_key)
        .env("RUST_LOG", "hail_api=info,hail_worker=info")
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
}

struct ChildProcess {
    name: &'static str,
    child: tokio::process::Child,
}

impl ChildProcess {
    fn spawn(name: &'static str, mut command: Command) -> Result<Self, std::io::Error> {
        Ok(Self {
            name,
            child: command.spawn()?,
        })
    }

    fn terminate(&mut self) {
        if let Some(id) = self.child.id() {
            let _ = std::process::Command::new("kill")
                .args(["-TERM", &id.to_string()])
                .status();
        }
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            match self.child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                Err(err) => {
                    eprintln!("failed waiting for {} shutdown: {err}", self.name);
                    break;
                }
            }
        }
        let _ = self.child.start_kill();
    }
}

fn allocate_free_port() -> Result<u16, std::io::Error> {
    let listener = TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))?;
    Ok(listener.local_addr()?.port())
}

fn target_dir() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let current_exe = std::env::current_exe()?;
    let deps_dir = current_exe.parent().ok_or("test executable parent")?;
    Ok(deps_dir
        .parent()
        .ok_or("target profile parent")?
        .to_path_buf())
}

async fn wait_for_ready(hail_url: &str) -> Result<(), String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(|err| err.to_string())?;
    let ready_url = format!("{hail_url}/readyz");
    for _ in 0..90 {
        if let Ok(response) = client.get(&ready_url).send().await
            && response.status().is_success()
        {
            return Ok(());
        }
        sleep(Duration::from_millis(500)).await;
    }
    Err(format!(
        "timed out waiting for hail API readiness at {ready_url}"
    ))
}

#[derive(Clone)]
struct ApiClient {
    base_url: String,
    cookie: String,
    client: Client,
}

impl ApiClient {
    async fn login(base_url: &str, email: &str, password: &str) -> Result<Self, String> {
        let client = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|err| err.to_string())?;
        let response = client
            .post(format!("{base_url}/api/auth/login"))
            .json(&json!({ "email": email, "password": password }))
            .send()
            .await
            .map_err(|err| err.to_string())?;
        if response.status() != StatusCode::OK {
            return Err(format!(
                "login returned {}: {}",
                response.status(),
                response.text().await.unwrap_or_default()
            ));
        }
        let cookie = response
            .headers()
            .get(reqwest::header::SET_COOKIE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .ok_or("login response missing Set-Cookie")?
            .to_owned();
        Ok(Self {
            base_url: base_url.to_owned(),
            cookie,
            client,
        })
    }

    async fn user_id(&self) -> Result<i64, String> {
        let json = self.get_json("/api/auth/me").await?;
        json["user"]["id"]
            .as_i64()
            .ok_or_else(|| format!("/api/auth/me missing user.id: {json}"))
    }

    async fn get_json(&self, path: &str) -> Result<Value, String> {
        let response = self
            .client
            .get(format!("{}{}", self.base_url, path))
            .header(reqwest::header::COOKIE, &self.cookie)
            .send()
            .await
            .map_err(|err| err.to_string())?;
        self.parse_json(response).await
    }

    async fn post_json(&self, path: &str, body: &Value) -> Result<Value, String> {
        let response = self
            .client
            .post(format!("{}{}", self.base_url, path))
            .header(reqwest::header::COOKIE, &self.cookie)
            .header(REQUEST_HEADER, "1")
            .json(body)
            .send()
            .await
            .map_err(|err| err.to_string())?;
        self.parse_json(response).await
    }

    async fn parse_json(&self, response: reqwest::Response) -> Result<Value, String> {
        let status = response.status();
        let text = response.text().await.map_err(|err| err.to_string())?;
        if !status.is_success() {
            return Err(format!("HTTP {status}: {text}"));
        }
        serde_json::from_str(&text).map_err(|err| format!("invalid JSON {err}: {text}"))
    }
}

async fn poll_json<F, Fut>(label: &str, timeout: Duration, mut f: F) -> Result<Value, String>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<Option<Value>, String>>,
{
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Some(value) = f().await? {
            return Ok(value);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!("timed out waiting for {label}"));
        }
        sleep(Duration::from_millis(500)).await;
    }
}
