#[path = "../src/gmail_client.rs"]
#[allow(dead_code)]
mod gmail_client;

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use base64::Engine;
use base64::prelude::BASE64_URL_SAFE_NO_PAD;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use gmail_client::{
    GmailApiErrorKind, GmailClient, GmailClientError, GmailRetryConfig, ListHistoryParams,
    ListMessagesParams, StaticGmailTokenSource, classify_gmail_error, parse_gmail_error,
    retry_after_duration,
};
use reqwest::header::{AUTHORIZATION, HeaderValue, RETRY_AFTER};
use reqwest::{Method, StatusCode};
use secrecy::SecretString;

#[derive(Clone, Debug)]
struct RequestRecord {
    method: Method,
    path: String,
    query: Option<String>,
    authorization: Option<String>,
}

#[derive(Clone, Debug)]
struct FakeResponse {
    status: StatusCode,
    retry_after: Option<u64>,
    body: serde_json::Value,
}

#[derive(Debug, Default)]
struct FakeState {
    requests: tokio::sync::Mutex<Vec<RequestRecord>>,
    profile_hits: AtomicUsize,
}

async fn fake_server() -> (String, Arc<FakeState>) {
    let state = Arc::new(FakeState::default());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server_state = state.clone();
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.expect("accept");
            let state = server_state.clone();
            tokio::spawn(async move {
                handle_connection(stream, state).await;
            });
        }
    });
    (format_base_url(addr), state)
}

fn format_base_url(addr: SocketAddr) -> String {
    format!("http://{addr}/gmail/v1/")
}

fn client(base_url: &str) -> GmailClient<StaticGmailTokenSource> {
    GmailClient::with_base_url(
        reqwest::Client::new(),
        StaticGmailTokenSource::new(SecretString::from("test-token")),
        base_url,
    )
    .expect("client")
    .with_retry_config(GmailRetryConfig {
        max_attempts: 3,
        base_delay: Duration::ZERO,
        max_delay: Duration::ZERO,
    })
}

async fn handle_connection(mut stream: TcpStream, state: Arc<FakeState>) {
    let mut buffer = vec![0_u8; 8192];
    let read = stream.read(&mut buffer).await.expect("read request");
    let request = String::from_utf8_lossy(&buffer[..read]);
    let mut lines = request.lines();
    let request_line = lines.next().expect("request line");
    let mut parts = request_line.split_whitespace();
    let method = Method::from_bytes(parts.next().expect("method").as_bytes()).expect("method");
    let target = parts.next().expect("target");
    let (path, query) = target
        .split_once('?')
        .map_or((target.to_owned(), None), |(path, query)| {
            (path.to_owned(), Some(query.to_owned()))
        });
    let authorization = lines.find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case(AUTHORIZATION.as_str())
            .then(|| value.trim().to_owned())
    });

    state.requests.lock().await.push(RequestRecord {
        method,
        path: path.clone(),
        query: query.clone(),
        authorization,
    });

    let query = parse_query(query.as_deref());
    let response = route_request(&state, &path, &query).await;
    let body = response.body.to_string();
    let mut header = format!(
        "HTTP/1.1 {} {}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n",
        response.status.as_u16(),
        response.status.canonical_reason().unwrap_or("Unknown"),
        body.len()
    );
    if let Some(seconds) = response.retry_after {
        header.push_str(&format!("{}: {}\r\n", RETRY_AFTER.as_str(), seconds));
    }
    header.push_str("\r\n");
    stream
        .write_all(header.as_bytes())
        .await
        .expect("write headers");
    stream.write_all(body.as_bytes()).await.expect("write body");
}

fn parse_query(query: Option<&str>) -> std::collections::HashMap<String, String> {
    query
        .into_iter()
        .flat_map(|query| query.split('&'))
        .filter_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            Some((key.to_owned(), value.to_owned()))
        })
        .collect()
}

async fn route_request(
    state: &FakeState,
    path: &str,
    query: &std::collections::HashMap<String, String>,
) -> FakeResponse {
    match path {
        "/gmail/v1/users/me/profile" => profile(state).await,
        "/gmail/v1/users/me/messages" => list_messages(query),
        "/gmail/v1/users/me/history" => list_history(query),
        path if path.starts_with("/gmail/v1/users/me/messages/") => get_message(path),
        _ => FakeResponse {
            status: StatusCode::NOT_FOUND,
            retry_after: None,
            body: json!({"error":{"message":"not found"}}),
        },
    }
}

async fn profile(state: &FakeState) -> FakeResponse {
    let hit = state.profile_hits.fetch_add(1, Ordering::SeqCst);
    if hit == 0 {
        return FakeResponse {
            status: StatusCode::TOO_MANY_REQUESTS,
            retry_after: Some(0),
            body: json!({"error":{"message":"slow down","errors":[{"reason":"rateLimitExceeded"}]}}),
        };
    }
    FakeResponse {
        status: StatusCode::OK,
        retry_after: None,
        body: json!({
            "emailAddress": "user@gmail.example",
            "messagesTotal": 10,
            "threadsTotal": 9,
            "historyId": "12345"
        }),
    }
}

fn list_messages(query: &std::collections::HashMap<String, String>) -> FakeResponse {
    match query.get("pageToken").map(String::as_str) {
        Some("page-2") => FakeResponse {
            status: StatusCode::OK,
            retry_after: None,
            body: json!({"messages":[{"id":"m2","threadId":"t2"}]}),
        },
        Some("loop") => FakeResponse {
            status: StatusCode::OK,
            retry_after: None,
            body: json!({"messages":[{"id":"looped"}],"nextPageToken":"loop"}),
        },
        _ => FakeResponse {
            status: StatusCode::OK,
            retry_after: None,
            body: json!({
                "messages":[{"id":"m1","threadId":"t1"}],
                "nextPageToken":"page-2",
                "resultSizeEstimate":2
            }),
        },
    }
}

fn list_history(query: &std::collections::HashMap<String, String>) -> FakeResponse {
    assert_eq!(query.get("startHistoryId").map(String::as_str), Some("100"));
    match query.get("pageToken").map(String::as_str) {
        Some("hist-2") => FakeResponse {
            status: StatusCode::OK,
            retry_after: None,
            body: json!({
                "history":[{"id":"102","messagesAdded":[{"message":{"id":"m2","threadId":"t2"}}]}],
                "historyId":"103"
            }),
        },
        _ => FakeResponse {
            status: StatusCode::OK,
            retry_after: None,
            body: json!({
                "history":[{"id":"101","messagesAdded":[{"message":{"id":"m1","threadId":"t1"}}]}],
                "nextPageToken":"hist-2"
            }),
        },
    }
}

fn get_message(path: &str) -> FakeResponse {
    let id = path.rsplit('/').next().expect("id").to_owned();
    match id.as_str() {
        "missing-raw" => FakeResponse {
            status: StatusCode::OK,
            retry_after: None,
            body: json!({"id":"missing-raw"}),
        },
        "denied" => FakeResponse {
            status: StatusCode::FORBIDDEN,
            retry_after: None,
            body: json!({
                "error":{
                    "message":"forbidden",
                    "errors":[{"reason":"insufficientPermissions","message":"no scope"}]
                }
            }),
        },
        _ => FakeResponse {
            status: StatusCode::OK,
            retry_after: None,
            body: json!({
                "id": id,
                "threadId":"thread-1",
                "historyId":"42",
                "raw": BASE64_URL_SAFE_NO_PAD.encode(b"Subject: hi\r\n\r\nBody")
            }),
        },
    }
}

#[tokio::test]
async fn profile_retries_rate_limit_and_sends_bearer_token() {
    let (base_url, state) = fake_server().await;
    let profile = client(&base_url).profile().await.expect("profile");
    assert_eq!(profile.email_address, "user@gmail.example");
    assert_eq!(profile.history_id.as_deref(), Some("12345"));

    let requests = state.requests.lock().await;
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|request| request.method == Method::GET));
    assert!(
        requests
            .iter()
            .all(|request| request.path == "/gmail/v1/users/me/profile")
    );
    assert!(
        requests
            .iter()
            .all(|request| request.authorization.as_deref() == Some("Bearer test-token"))
    );
}

#[tokio::test]
async fn list_all_messages_pages_and_encodes_query() {
    let (base_url, state) = fake_server().await;
    let messages = client(&base_url)
        .list_all_messages(&ListMessagesParams {
            max_results: Some(999),
            page_token: None,
            query: Some("after:2024/01/01".to_owned()),
            label_ids: vec!["INBOX".to_owned(), "SENT".to_owned()],
            include_spam_trash: true,
        })
        .await
        .expect("messages");

    assert_eq!(
        messages.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["m1", "m2"]
    );
    let requests = state.requests.lock().await;
    let first_query = requests[0].query.as_deref().expect("query");
    assert!(first_query.contains("maxResults=500"), "{first_query}");
    assert!(
        first_query.contains("q=after%3A2024%2F01%2F01"),
        "{first_query}"
    );
    assert!(first_query.contains("labelIds=INBOX"), "{first_query}");
    assert!(first_query.contains("labelIds=SENT"), "{first_query}");
    assert!(
        first_query.contains("includeSpamTrash=true"),
        "{first_query}"
    );
    let second_query = requests[1].query.as_deref().expect("query");
    assert!(second_query.contains("pageToken=page-2"), "{second_query}");
}

#[tokio::test]
async fn pagination_loop_is_detected() {
    let (base_url, _state) = fake_server().await;
    let error = client(&base_url)
        .list_all_messages(&ListMessagesParams {
            page_token: Some("loop".to_owned()),
            ..ListMessagesParams::default()
        })
        .await
        .expect_err("pagination loop");

    assert!(matches!(
        error,
        GmailClientError::PaginationLoop { page_token } if page_token == "loop"
    ));
}

#[tokio::test]
async fn list_history_pages_and_encodes_filters() {
    let (base_url, state) = fake_server().await;
    let first = client(&base_url)
        .list_history(&ListHistoryParams {
            start_history_id: "100".to_owned(),
            max_results: Some(999),
            page_token: None,
            label_id: Some("INBOX".to_owned()),
            history_types: vec!["messageAdded".to_owned()],
        })
        .await
        .expect("history");
    assert_eq!(first.history[0].id, "101");
    assert_eq!(first.next_page_token.as_deref(), Some("hist-2"));

    let second = client(&base_url)
        .list_history(&ListHistoryParams {
            start_history_id: "100".to_owned(),
            max_results: Some(50),
            page_token: first.next_page_token,
            label_id: Some("INBOX".to_owned()),
            history_types: vec!["messageAdded".to_owned()],
        })
        .await
        .expect("history page 2");
    assert_eq!(second.history_id.as_deref(), Some("103"));

    let requests = state.requests.lock().await;
    let first_query = requests[0].query.as_deref().expect("query");
    assert!(first_query.contains("startHistoryId=100"), "{first_query}");
    assert!(first_query.contains("maxResults=500"), "{first_query}");
    assert!(first_query.contains("labelId=INBOX"), "{first_query}");
    assert!(
        first_query.contains("historyTypes=messageAdded"),
        "{first_query}"
    );
    let second_query = requests[1].query.as_deref().expect("query");
    assert!(second_query.contains("pageToken=hist-2"), "{second_query}");
}

#[tokio::test]
async fn get_raw_message_decodes_rfc822() {
    let (base_url, state) = fake_server().await;
    let message = client(&base_url)
        .get_raw_message("msg-1")
        .await
        .expect("message");
    assert_eq!(message.id, "msg-1");
    assert_eq!(message.thread_id.as_deref(), Some("thread-1"));
    assert_eq!(message.history_id.as_deref(), Some("42"));
    assert_eq!(message.rfc822, b"Subject: hi\r\n\r\nBody");

    let requests = state.requests.lock().await;
    assert_eq!(requests[0].path, "/gmail/v1/users/me/messages/msg-1");
    assert_eq!(requests[0].query.as_deref(), Some("format=raw"));
}

#[tokio::test]
async fn missing_raw_is_mapped() {
    let (base_url, _state) = fake_server().await;
    let error = client(&base_url)
        .get_raw_message("missing-raw")
        .await
        .expect_err("missing raw");
    assert!(matches!(error, GmailClientError::MissingRawMessage));
}

#[tokio::test]
async fn gmail_json_error_is_classified() {
    let (base_url, _state) = fake_server().await;
    let error = client(&base_url)
        .get_raw_message("denied")
        .await
        .expect_err("denied");

    assert!(matches!(
        error,
        GmailClientError::Api {
            status: StatusCode::FORBIDDEN,
            kind: GmailApiErrorKind::PermissionDenied,
            reason: Some(ref reason),
            ref message,
            retry_after: None,
        } if reason == "insufficientPermissions" && message == "forbidden"
    ));
}

#[test]
fn retry_after_parses_delta_seconds_only() {
    let value = HeaderValue::from_static("7");
    assert_eq!(
        retry_after_duration(Some(&value)),
        Some(Duration::from_secs(7))
    );
    let date = HeaderValue::from_static("Wed, 21 Oct 2015 07:28:00 GMT");
    assert_eq!(retry_after_duration(Some(&date)), None);
}

#[test]
fn error_classification_handles_common_gmail_cases() {
    assert_eq!(
        classify_gmail_error(StatusCode::FORBIDDEN, Some("rateLimitExceeded")),
        GmailApiErrorKind::RateLimited
    );
    assert_eq!(
        classify_gmail_error(StatusCode::SERVICE_UNAVAILABLE, None),
        GmailApiErrorKind::Transient
    );
    assert_eq!(
        classify_gmail_error(StatusCode::UNAUTHORIZED, None),
        GmailApiErrorKind::Unauthorized
    );
}

#[tokio::test]
async fn for_each_message_page_visits_each_page() {
    let (base_url, _state) = fake_server().await;
    let mut pages = Vec::new();
    client(&base_url)
        .for_each_message_page(&ListMessagesParams::default(), |page| {
            pages.push(page.messages.len());
            std::future::ready(Ok(()))
        })
        .await
        .expect("pages");
    assert_eq!(pages, vec![1, 1]);
}

#[test]
fn parse_gmail_error_uses_envelope() {
    let parsed = parse_gmail_error(
        r#"{"error":{"message":"top","errors":[{"reason":"bad","message":"detail"}]}}"#,
    );
    assert_eq!(parsed.reason.as_deref(), Some("bad"));
    assert_eq!(parsed.message.as_deref(), Some("top"));
}
