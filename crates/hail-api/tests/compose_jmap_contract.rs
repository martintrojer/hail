use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Method, Request, StatusCode, header};
use hail_api::middleware::rate_limit::IpRateLimiter;
use hail_api::state::AppState;
use hail_core::{Config, KEY_LEN, parse_server_key};
use hail_jmap::jmap_client::{
    email::{EmailBodyPart, Property as EmailProperty},
    email_submission::Property as EmailSubmissionProperty,
    mailbox::{Role, query::Filter as MailboxFilter},
};
use hail_test::stalwart::{stalwart_tests_enabled, start_stalwart_fixture};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;

const HAIL_PASSWORD: &str = "hail-test-password";
const SERVER_KEY_HEX: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const REQUEST_HEADER: &str = "X-Hail-Request";
const SKIP_REASON: &str =
    "skipping compose JMAP/MIME contract integration test; set HAIL_RUN_STALWART_TESTS=1 to run it";

#[tokio::test]
async fn compose_draft_and_send_match_real_jmap_contract_when_enabled() {
    if !stalwart_tests_enabled() {
        eprintln!("{SKIP_REASON}");
        return;
    }

    let runtime = ContractRuntime::start()
        .await
        .expect("start Stalwart-backed compose contract runtime");
    let app = hail_api::build_router(runtime.state.clone(), false);
    let session = runtime
        .stalwart
        .login_seeded_user()
        .await
        .expect("seeded Stalwart user should login");
    let drafts_mailbox_id = role_mailbox_id(&session, Role::Drafts)
        .await
        .expect("Drafts mailbox should exist");

    let cookie = login(&app, runtime.stalwart.seeded_email(), HAIL_PASSWORD).await;
    let draft_id = create_autosave_draft(&app, &cookie).await;
    assert_draft_contract(&session, &draft_id, &drafts_mailbox_id).await;

    let send_json = post_json(
        &app,
        &cookie,
        "/api/compose",
        json!({
            "to": ["bob.compose-contract@example.net"],
            "cc": ["carol.compose-contract@example.net"],
            "bcc": ["dana.compose-contract@example.net"],
            "subject": "Compose contract send",
            "body_markdown": "Hello **Bob**.\n\n<script>alert('x')</script>\n\nRegards, Alice",
            "attachments": []
        }),
        StatusCode::OK,
    )
    .await;
    assert_eq!(send_json["status"], "sent");
    let sent_email_id = send_json["email_id"].as_str().expect("sent email_id");
    let submission_id = send_json["submission_id"]
        .as_str()
        .expect("submission_id should be present");

    assert_sent_email_contract(&session, sent_email_id, &drafts_mailbox_id).await;
    assert_submission_contract(&session, submission_id, sent_email_id).await;
}

struct ContractRuntime {
    _temp: TempDir,
    stalwart: hail_test::stalwart::StalwartFixture,
    state: AppState,
}

impl ContractRuntime {
    async fn start() -> Result<Self, Box<dyn std::error::Error>> {
        let temp = TempDir::new()?;
        let db_url = format!("sqlite://{}", temp.path().join("hail.db").display());
        let db = hail_db::connect(&db_url).await?;
        hail_db::migrate(&db).await?;
        let stalwart = start_stalwart_fixture().await?;

        unsafe {
            std::env::set_var("HAIL_DATABASE_URL", &db_url);
            std::env::set_var("HAIL_STALWART__JMAP_URL", stalwart.jmap_url());
            std::env::set_var("HAIL_SERVER__BIND", "127.0.0.1:0");
            std::env::set_var("HAIL_SERVER__PUBLIC_URL", "http://localhost");
            std::env::set_var("HAIL_SECRETS__SERVER_KEY", SERVER_KEY_HEX);
            std::env::remove_var("HAIL_ADMIN__EMAIL");
        }
        let config = Config::load_from(None)?;
        let key: [u8; KEY_LEN] = parse_server_key(&config.secrets.server_key)?;
        let state = AppState {
            db,
            config,
            server_key: Arc::new(key),
            login_limiter: Arc::new(IpRateLimiter::new(100, Duration::from_secs(60))),
            events: hail_api::events::AppEventBus::default(),
        };

        Ok(Self {
            _temp: temp,
            stalwart,
            state,
        })
    }
}

async fn login(app: &axum::Router, email: String, password: &str) -> String {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .extension(ConnectInfo(
                    "127.0.0.1:10000".parse::<std::net::SocketAddr>().unwrap(),
                ))
                .body(Body::from(
                    json!({ "email": email, "password": password }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .expect("login request should complete");
    assert_eq!(response.status(), StatusCode::OK);
    response
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .expect("login response Set-Cookie")
        .to_owned()
}

async fn create_autosave_draft(app: &axum::Router, cookie: &str) -> String {
    let json = post_json(
        app,
        cookie,
        "/api/drafts",
        json!({
            "to": ["draft-recipient.compose-contract@example.net"],
            "cc": ["draft-cc.compose-contract@example.net"],
            "bcc": ["draft-bcc.compose-contract@example.net"],
            "subject": "Compose contract draft",
            "body_markdown": "Draft body with *markdown* and Bcc.",
            "attachments": []
        }),
        StatusCode::CREATED,
    )
    .await;
    json["draft_id"].as_str().expect("draft_id").to_owned()
}

async fn post_json(
    app: &axum::Router,
    cookie: &str,
    path: &str,
    body: Value,
    expected_status: StatusCode,
) -> Value {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(path)
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::COOKIE, cookie)
                .header(REQUEST_HEADER, "1")
                .extension(ConnectInfo(
                    "127.0.0.1:10001".parse::<std::net::SocketAddr>().unwrap(),
                ))
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .expect("API request should complete");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("response body")
        .to_bytes();
    let text = String::from_utf8_lossy(&bytes);
    assert_eq!(status, expected_status, "unexpected response body: {text}");
    serde_json::from_slice(&bytes).expect("JSON response")
}

async fn role_mailbox_id(session: &hail_jmap::Session, role: Role) -> Result<String, String> {
    let mut response = session
        .client()
        .mailbox_query(Some(MailboxFilter::role(role)), None::<Vec<_>>)
        .await
        .map_err(|err| err.to_string())?;
    response
        .take_ids()
        .into_iter()
        .next()
        .ok_or_else(|| "role mailbox not found".to_owned())
}

async fn assert_draft_contract(
    session: &hail_jmap::Session,
    draft_id: &str,
    drafts_mailbox_id: &str,
) {
    let email = fetch_email(session, draft_id).await;
    assert_eq!(email.subject(), Some("Compose contract draft"));
    assert_eq!(addresses(email.from()), vec!["alice@hail.test"]);
    assert_eq!(
        addresses(email.to()),
        vec!["draft-recipient.compose-contract@example.net"]
    );
    assert_eq!(
        addresses(email.cc()),
        vec!["draft-cc.compose-contract@example.net"]
    );
    assert_eq!(
        addresses(email.bcc()),
        vec!["draft-bcc.compose-contract@example.net"]
    );
    assert!(email.keywords().contains(&"$draft"));
    assert!(email.mailbox_ids().contains(&drafts_mailbox_id));
    assert_text_body(&email, "Draft body with *markdown* and Bcc.");
}

async fn assert_sent_email_contract(
    session: &hail_jmap::Session,
    email_id: &str,
    drafts_mailbox_id: &str,
) {
    let email = fetch_email(session, email_id).await;
    assert_eq!(email.subject(), Some("Compose contract send"));
    assert_eq!(addresses(email.from()), vec!["alice@hail.test"]);
    assert_eq!(
        addresses(email.to()),
        vec!["bob.compose-contract@example.net"]
    );
    assert_eq!(
        addresses(email.cc()),
        vec!["carol.compose-contract@example.net"]
    );
    assert_eq!(
        addresses(email.bcc()),
        vec!["dana.compose-contract@example.net"]
    );
    assert!(email.keywords().contains(&"$draft"));
    assert!(email.mailbox_ids().contains(&drafts_mailbox_id));
    assert_text_body(&email, "Hello Bob");
    assert_html_body(&email, "<strong>Bob</strong>");
    assert_html_body_absent(&email, "<script>");
    assert_html_body_absent(&email, "alert('x')");
}

async fn assert_submission_contract(
    session: &hail_jmap::Session,
    submission_id: &str,
    email_id: &str,
) {
    let submission = session
        .client()
        .email_submission_get(
            submission_id,
            Some(vec![
                EmailSubmissionProperty::Id,
                EmailSubmissionProperty::EmailId,
                EmailSubmissionProperty::Envelope,
            ]),
        )
        .await
        .expect("EmailSubmission/get should succeed")
        .expect("submission should exist");
    assert_eq!(submission.email_id(), Some(email_id));
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
            "bob.compose-contract@example.net",
            "carol.compose-contract@example.net",
            "dana.compose-contract@example.net",
        ]
    );
}

async fn fetch_email(
    session: &hail_jmap::Session,
    email_id: &str,
) -> hail_jmap::jmap_client::email::Email {
    let mut request = session.client().build();
    request
        .get_email()
        .ids([email_id])
        .properties([
            EmailProperty::Id,
            EmailProperty::MailboxIds,
            EmailProperty::Keywords,
            EmailProperty::From,
            EmailProperty::To,
            EmailProperty::Cc,
            EmailProperty::Bcc,
            EmailProperty::Subject,
            EmailProperty::TextBody,
            EmailProperty::HtmlBody,
            EmailProperty::BodyValues,
        ])
        .arguments()
        .fetch_text_body_values(true)
        .fetch_html_body_values(true)
        .max_body_value_bytes(64 * 1024);
    request
        .send_get_email()
        .await
        .expect("Email/get should succeed")
        .take_list()
        .pop()
        .expect("email should exist")
}

fn addresses(addresses: Option<&[hail_jmap::jmap_client::email::EmailAddress]>) -> Vec<String> {
    addresses
        .unwrap_or_default()
        .iter()
        .map(|address| address.email().to_owned())
        .collect()
}

fn assert_text_body(email: &hail_jmap::jmap_client::email::Email, expected: &str) {
    let body = first_body_value(email.text_body(), email).expect("text body value");
    assert!(
        body.contains(expected),
        "text body missing {expected:?}: {body:?}"
    );
}

fn assert_html_body(email: &hail_jmap::jmap_client::email::Email, expected: &str) {
    let body = first_body_value(email.html_body(), email).expect("html body value");
    assert!(
        body.contains(expected),
        "html body missing {expected:?}: {body:?}"
    );
}

fn assert_html_body_absent(email: &hail_jmap::jmap_client::email::Email, unexpected: &str) {
    let body = first_body_value(email.html_body(), email).expect("html body value");
    assert!(
        !body.contains(unexpected),
        "html body unexpectedly contained {unexpected:?}: {body:?}"
    );
}

fn first_body_value(
    parts: Option<&[EmailBodyPart]>,
    email: &hail_jmap::jmap_client::email::Email,
) -> Option<String> {
    parts?
        .first()?
        .part_id()
        .and_then(|part_id| email.body_value(part_id))
        .map(|value| value.value().to_owned())
}
