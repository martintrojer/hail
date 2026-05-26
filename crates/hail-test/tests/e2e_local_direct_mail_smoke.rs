use hail_test::{
    local_mail_testbed::import_raw_message_via_jmap,
    stalwart::{StalwartFixture, start_stalwart_fixture_unchecked},
};
use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use std::{
    future::Future,
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener},
    path::PathBuf,
    process::Stdio,
    time::Duration,
};
use tempfile::TempDir;
use tokio::{process::Command, time::sleep};

const SMOKE_FIXTURE_NAME: &str = "personal-simple.e2e-local-direct-mail-smoke.eml";
const SMOKE_SENDER: &str = "maya.e2e-local-direct-mail-smoke@personal.example";
const SMOKE_SUBJECT: &str = "E2E local direct mail smoke: Dinner on Thursday?";
const SMOKE_REPLY_FIXTURE_NAME: &str = "personal-thread-reply.e2e-local-direct-mail-smoke.eml";
const SMOKE_REPLY_SUBJECT: &str = "Re: E2E local direct mail smoke: Dinner on Thursday?";
const HAIL_PASSWORD: &str = "hail-test-password";
const SERVER_KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const REQUEST_HEADER: &str = "X-Hail-Request";
const DIRECT_ENV: &str = "HAIL_E2E_LOCAL_DIRECT_MAIL_SMOKE_DIRECT";
const SKIP_REASON: &str = "skipping local/direct mail smoke; set HAIL_RUN_LOCAL_MAIL_TESTBED=1 to run scripts/e2e-local-direct-mail-smoke.sh";

fn local_mail_testbed_enabled() -> bool {
    std::env::var("HAIL_RUN_LOCAL_MAIL_TESTBED").is_ok_and(|value| value == "1")
}

fn direct_smoke_enabled() -> bool {
    std::env::var(DIRECT_ENV).is_ok_and(|value| value == "1")
}

#[test]
fn local_direct_mail_smoke_script_wrapper() {
    if !local_mail_testbed_enabled() {
        eprintln!("{SKIP_REASON}");
        return;
    }
    if direct_smoke_enabled() {
        eprintln!("skipping script wrapper inside direct script-driven cargo invocation");
        return;
    }

    let status = std::process::Command::new("scripts/e2e-local-direct-mail-smoke.sh")
        .env("HAIL_RUN_LOCAL_MAIL_TESTBED", "1")
        .status()
        .expect("run local/direct mail smoke script");
    assert!(status.success(), "script exited with {status}");
}

#[tokio::test]
async fn local_direct_mail_smoke_flow_when_enabled() {
    if !local_mail_testbed_enabled() || !direct_smoke_enabled() {
        eprintln!("{SKIP_REASON}");
        return;
    }

    let smoke = SmokeRuntime::start()
        .await
        .expect("start local/direct mail smoke runtime");

    let api = ApiClient::login(
        smoke.hail_url(),
        &smoke.stalwart.seeded_email(),
        HAIL_PASSWORD,
    )
    .await
    .expect("login to hail API");

    // The worker only supervises users with active hail sessions. Give its
    // 1-second smoke-test tick enough time to see the freshly-created session
    // and seed JMAP cursors before injecting mail that must be routed.
    sleep(Duration::from_secs(2)).await;

    let session = smoke
        .stalwart
        .login_seeded_user()
        .await
        .expect("seeded Stalwart user should login");
    let first_import = import_raw_message_via_jmap(
        &session,
        SMOKE_FIXTURE_NAME,
        smoke_message_bytes(SMOKE_SENDER, SMOKE_SUBJECT, None),
    )
    .await
    .expect("import smoke message through JMAP Email/import");
    assert!(!first_import.email_id.is_empty());

    let pending = poll_json("screener pending sender", Duration::from_secs(45), || {
        let api = api.clone();
        async move {
            let json = api.get_json("/api/views/screener").await?;
            let contains = json["senders"].as_array().is_some_and(|senders| {
                senders
                    .iter()
                    .any(|sender| sender["sender"].as_str() == Some(SMOKE_SENDER))
            });
            Ok(contains.then_some(json))
        }
    })
    .await
    .expect("screener pending sender should appear");
    assert!(
        pending["senders"]
            .as_array()
            .unwrap()
            .iter()
            .any(|sender| { sender["message_count"].as_i64().unwrap_or_default() >= 1 })
    );

    let decision = api
        .post_json(
            "/api/screener/decisions",
            &json!({
                "sender": SMOKE_SENDER,
                "decision": "approve",
                "classify_as": "imbox",
                "apply_to_history": false
            }),
        )
        .await
        .expect("approve sender");
    assert_eq!(decision["sender"], SMOKE_SENDER);
    assert_eq!(decision["decision"], "approve");
    assert_eq!(decision["classify_as"], "imbox");

    let reply_import = import_raw_message_via_jmap(
        &session,
        SMOKE_REPLY_FIXTURE_NAME,
        smoke_message_bytes(
            SMOKE_SENDER,
            SMOKE_REPLY_SUBJECT,
            Some("<e2e-local-direct-mail-smoke-1@personal.example>"),
        ),
    )
    .await
    .expect("import second smoke message through JMAP Email/import");
    assert!(!reply_import.email_id.is_empty());

    let imbox_item = poll_json("imbox smoke message", Duration::from_secs(45), || {
        let api = api.clone();
        async move {
            let json = api.get_json("/api/views/imbox?limit=10").await?;
            Ok(json["items"].as_array().and_then(|items| {
                items
                    .iter()
                    .find(|item| item["subject"].as_str() == Some(SMOKE_REPLY_SUBJECT))
                    .cloned()
            }))
        }
    })
    .await
    .expect("approved sender message should appear in Imbox");
    assert_eq!(imbox_item["from"], format!("Maya Smoke <{SMOKE_SENDER}>"));
    assert_eq!(imbox_item["classification"], "imbox");

    let thread_id = imbox_item["thread_id"].as_str().expect("thread_id");
    let thread = api
        .get_json(&format!("/api/threads/{thread_id}"))
        .await
        .expect("thread view should load");
    assert_eq!(thread["thread_id"], thread_id);
    let messages = thread["messages"].as_array().expect("messages");
    assert!(
        thread["subject"].as_str().is_some_and(
            |subject| subject.contains("E2E local direct mail smoke: Dinner on Thursday?")
        ),
        "thread subject should describe the smoke conversation: {thread}"
    );
    assert!(
        messages.iter().any(|message| message["preview"]
            .as_str()
            .is_some_and(|preview| preview.contains("Smoke test reply after approval"))),
        "thread response should include smoke reply preview: {thread}"
    );
}

fn smoke_message_bytes(sender: &str, subject: &str, in_reply_to: Option<&str>) -> Vec<u8> {
    let threading_headers = in_reply_to.map_or_else(String::new, |message_id| {
        format!("In-Reply-To: {message_id}\r\nReferences: {message_id}\r\n")
    });
    let message_id = if in_reply_to.is_some() {
        "<e2e-local-direct-mail-smoke-2@personal.example>"
    } else {
        "<e2e-local-direct-mail-smoke-1@personal.example>"
    };
    let body = if in_reply_to.is_some() {
        "Smoke test reply after approval. This message should route to the Imbox."
    } else {
        "Smoke test first message. This sender should appear in the Screener."
    };

    format!(
        "From: Maya Smoke <{sender}>\r\n\
To: Alice Test <alice@hail.test>\r\n\
Subject: {subject}\r\n\
Date: Tue, 20 May 2025 18:42:11 -0700\r\n\
Message-ID: {message_id}\r\n\
{threading_headers}\
MIME-Version: 1.0\r\n\
Content-Type: text/html; charset=UTF-8\r\n\
\r\n\
<!doctype html><html><body><p>{body}</p></body></html>\r\n"
    )
    .into_bytes()
}

struct SmokeRuntime {
    _temp: TempDir,
    stalwart: StalwartFixture,
    api: ChildProcess,
    _worker: ChildProcess,
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
        let target_dir = target_dir()?;

        let mut api = Command::new(target_dir.join("hail-api"));
        configure_process(&mut api, &db_url, &jmap_url, &hail_url, SERVER_KEY)
            .env("HAIL_SERVER__BIND", format!("127.0.0.1:{api_port}"));
        let api = ChildProcess::spawn("hail-api", api)?;
        wait_for_ready(&hail_url).await?;

        let mut worker = Command::new(target_dir.join("hail-worker"));
        configure_process(&mut worker, &db_url, &jmap_url, &hail_url, SERVER_KEY)
            .env("HAIL_TICK_SECS", "1")
            .env("HAIL_IMPORT_CATCHUP_SECS", "1")
            .env("HAIL_RECONCILE_EVERY_SECS", "3600");
        let worker = ChildProcess::spawn("hail-worker", worker)?;

        Ok(Self {
            _temp: temp,
            stalwart,
            api,
            _worker: worker,
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
        self._worker.terminate();
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
