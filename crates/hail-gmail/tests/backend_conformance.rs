use async_trait::async_trait;
use base64::Engine;
use base64::prelude::BASE64_URL_SAFE_NO_PAD;
use bytes::Bytes;
use hail_backend::conformance::{
    MailBackendConformance, PrincipalExpectation, run_mail_backend_conformance,
};
use hail_backend::{
    BackendMsgId, BlobRef, Capabilities, Change, Envelope, Keyword, Mailbox, MailboxRole,
    RawMessage, SubmissionId, SyncCursor, MailBackend,
};
use hail_gmail::GmailBackend;
use hail_gmail::gmail_client::{GmailClient, GmailRetryConfig, StaticGmailTokenSource};
use hail_gmail::gmail_outbound_smtp::{
    GmailOutboundMessage, GmailOutboundSmtpClient, GmailOutboundSmtpError, GmailRawOutboundMessage,
    GmailSmtpSender, GmailSmtpSubmission,
};
use reqwest::StatusCode;
use secrecy::SecretString;
use serde_json::json;
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const GMAIL_CAPABILITIES: Capabilities = Capabilities {
    supports_initial_import: true,
    supports_eventsource: false,
    supports_principals_admin: false,
    supports_send: true,
    native_threading: true,
    max_attachment_size: 25 * 1024 * 1024,
    label_path_separator: '/',
};

#[derive(Clone, Debug)]
struct RequestRecord {
    method: String,
    path: String,
    body: String,
}

#[derive(Debug, Default)]
struct FakeGmailState {
    requests: tokio::sync::Mutex<Vec<RequestRecord>>,
}

#[derive(Clone, Default)]
struct CapturingSmtpSender {
    captured: Arc<tokio::sync::Mutex<Vec<GmailRawOutboundMessage>>>,
}

#[async_trait]
impl GmailSmtpSender for CapturingSmtpSender {
    async fn send_message(
        &self,
        _access_token: SecretString,
        _message: &GmailOutboundMessage,
    ) -> Result<(), GmailOutboundSmtpError> {
        unreachable!("MailBackend conformance sends raw RFC822 bytes")
    }

    async fn send_raw_message(
        &self,
        _access_token: SecretString,
        message: &GmailRawOutboundMessage,
    ) -> Result<GmailSmtpSubmission, GmailOutboundSmtpError> {
        self.captured.lock().await.push(message.clone());
        Ok(GmailSmtpSubmission {
            id: "smtp-queued-1".to_string(),
        })
    }
}

#[tokio::test]
async fn gmail_backend_satisfies_shared_mail_backend_conformance() {
    let (base_url, state) = fake_gmail_server().await;
    let token_source = StaticGmailTokenSource::new(SecretString::from("test-token"));
    let gmail = GmailClient::with_base_url(reqwest::Client::new(), token_source.clone(), &base_url)
        .expect("gmail client")
        .with_retry_config(no_retries());
    let smtp_sender = CapturingSmtpSender::default();
    let smtp_captured = Arc::clone(&smtp_sender.captured);
    let smtp = GmailOutboundSmtpClient::new(token_source, smtp_sender);
    let backend = GmailBackend::from_parts(gmail, smtp);
    let fixture = gmail_fixture();

    run_mail_backend_conformance(&backend, &fixture).await;

    let sent = smtp_captured.lock().await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].mail_from, fixture.send_envelope.mail_from);
    assert_eq!(sent[0].rcpt_to, fixture.send_envelope.rcpt_to);
    assert_eq!(sent[0].rfc822, fixture.send_rfc822);
    drop(sent);

    assert_recorded_requests_cover_every_gmail_rest_endpoint(&state).await;
}

#[tokio::test]
async fn gmail_backend_sets_unread_by_adding_gmail_unread_label() {
    let (base_url, state) = fake_gmail_server().await;
    let token_source = StaticGmailTokenSource::new(SecretString::from("test-token"));
    let gmail = GmailClient::with_base_url(reqwest::Client::new(), token_source.clone(), &base_url)
        .expect("gmail client")
        .with_retry_config(no_retries());
    let smtp = GmailOutboundSmtpClient::new(token_source, CapturingSmtpSender::default());
    let backend = GmailBackend::from_parts(gmail, smtp);

    backend
        .set_keywords(
            &BackendMsgId::new("msg-mark-unread"),
            &[],
            &[Keyword::new("$seen")],
        )
        .await
        .expect("mark unread");

    assert_eq!(
        last_modify_body(&state).await,
        json!({"addLabelIds":["UNREAD"],"removeLabelIds":[]})
    );
}

fn gmail_fixture() -> MailBackendConformance {
    let message_id = BackendMsgId::new("msg-1");
    let blob_ref = BlobRef::new("msg-1:att-1");
    let rfc822 = Bytes::from_static(
        b"From: sender@example.org\r\nTo: user@example.org\r\nSubject: Gmail fixture\r\n\r\nHello from Gmail",
    );
    MailBackendConformance {
        expected_capabilities: GMAIL_CAPABILITIES,
        listed_message_ids: vec![message_id.clone()],
        listed_next_cursor: Some("page-2".to_string()),
        message_id: message_id.clone(),
        expected_message: RawMessage {
            id: message_id.clone(),
            thread_id: Some("thread-1".to_string()),
            rfc822,
            keywords: vec![Keyword::new("INBOX")],
            envelope: None,
            received_at_epoch_secs: None,
            size_bytes: None,
            blob_refs: Vec::new(),
            attachments: Vec::new(),
            metadata: BTreeMap::from([("gmail_history_id".to_string(), "hist-msg-1".to_string())]),
        },
        blob_ref,
        expected_blob: Bytes::from_static(b"attachment bytes"),
        keyword_additions: vec![Keyword::new("$flagged"), Keyword::new("$seen")],
        keyword_removals: vec![Keyword::new("$draft")],
        move_role: MailboxRole::Trash,
        send_rfc822: b"From: sender@example.org\r\nTo: user@example.org\r\nSubject: conformance send\r\n\r\nBody".to_vec(),
        send_envelope: Envelope {
            mail_from: "sender@example.org".to_string(),
            rcpt_to: vec!["user@example.org".to_string()],
        },
        expected_submission_id: SubmissionId::new("smtp-queued-1"),
        poll_cursor: SyncCursor::new("hist-0"),
        expected_changes: vec![
            Change::MessageCreated {
                id: BackendMsgId::new("msg-2"),
                raw_ref: None,
            },
            Change::MailboxRoleChanged {
                id: BackendMsgId::new("msg-1"),
                role: MailboxRole::Trash,
            },
            Change::MessageUpdated {
                id: BackendMsgId::new("msg-3"),
                keywords: None,
                keywords_added: Vec::new(),
                keywords_removed: vec![Keyword::new("$seen")],
            },
            Change::MessageUpdated {
                id: BackendMsgId::new("msg-1"),
                keywords: None,
                keywords_added: vec![Keyword::new("$seen")],
                keywords_removed: Vec::new(),
            },
        ],
        expected_next_cursor: SyncCursor::new("hist-2"),
        expected_mailboxes: vec![
            Mailbox {
                id: "INBOX".to_string(),
                name: "INBOX".to_string(),
                role: MailboxRole::Inbox,
                parent_id: None,
            },
            Mailbox {
                id: "TRASH".to_string(),
                name: "TRASH".to_string(),
                role: MailboxRole::Trash,
                parent_id: None,
            },
            Mailbox {
                id: "Label_1".to_string(),
                name: "Projects".to_string(),
                role: MailboxRole::Custom,
                parent_id: None,
            },
        ],
        principals: PrincipalExpectation::Unsupported,
    }
}

fn no_retries() -> GmailRetryConfig {
    GmailRetryConfig {
        max_attempts: 1,
        base_delay: std::time::Duration::ZERO,
        max_delay: std::time::Duration::ZERO,
    }
}

async fn fake_gmail_server() -> (String, Arc<FakeGmailState>) {
    let state = Arc::new(FakeGmailState::default());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server_state = Arc::clone(&state);
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let state = Arc::clone(&server_state);
            tokio::spawn(async move {
                handle_connection(stream, state).await;
            });
        }
    });
    (format!("http://{addr}/gmail/v1/"), state)
}

async fn handle_connection(mut stream: TcpStream, state: Arc<FakeGmailState>) {
    let mut buffer = vec![0_u8; 16 * 1024];
    let read = stream.read(&mut buffer).await.expect("read request");
    let request = String::from_utf8_lossy(&buffer[..read]);
    let (headers, body) = request.split_once("\r\n\r\n").unwrap_or((&request, ""));
    let mut lines = headers.lines();
    let request_line = lines.next().expect("request line");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().expect("method").to_string();
    let target = parts.next().expect("target");
    let (path, query) = target
        .split_once('?')
        .map_or((target.to_string(), None), |(path, query)| {
            (path.to_string(), Some(query.to_string()))
        });
    state.requests.lock().await.push(RequestRecord {
        method: method.clone(),
        path: path.clone(),
        body: body.to_string(),
    });

    let response = route(&method, &path, query.as_deref(), body);
    let response_body = response.body.to_string();
    let headers = format!(
        "HTTP/1.1 {} {}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        response.status.as_u16(),
        response.status.canonical_reason().unwrap_or("OK"),
        response_body.len()
    );
    stream
        .write_all(headers.as_bytes())
        .await
        .expect("write headers");
    stream
        .write_all(response_body.as_bytes())
        .await
        .expect("write body");
}

struct FakeResponse {
    status: StatusCode,
    body: serde_json::Value,
}

fn route(method: &str, path: &str, query: Option<&str>, body: &str) -> FakeResponse {
    match (method, path) {
        ("GET", "/gmail/v1/users/me/messages") => {
            assert!(query.unwrap_or_default().contains("maxResults=10"));
            ok(json!({
                "messages": [{"id": "msg-1", "threadId": "thread-1"}],
                "nextPageToken": "page-2",
                "resultSizeEstimate": 1
            }))
        }
        ("GET", "/gmail/v1/users/me/messages/msg-1") => match query {
            Some("format=raw") => ok(json!({
                "id": "msg-1",
                "threadId": "thread-1",
                "historyId": "hist-msg-1",
                "labelIds": ["INBOX", "UNREAD"],
                "raw": BASE64_URL_SAFE_NO_PAD.encode(
                    b"From: sender@example.org\r\nTo: user@example.org\r\nSubject: Gmail fixture\r\n\r\nHello from Gmail"
                )
            })),
            Some("format=full") => ok(json!({
                "id": "msg-1",
                "threadId": "thread-1",
                "historyId": "hist-msg-1",
                "labelIds": ["INBOX", "UNREAD"],
                "payload": {
                    "mimeType": "text/plain",
                    "filename": "",
                    "body": {"size": 16}
                }
            })),
            other => panic!("unexpected get message query {other:?}"),
        },
        ("GET", "/gmail/v1/users/me/messages/msg-1/attachments/att-1") => ok(json!({
            "data": BASE64_URL_SAFE_NO_PAD.encode(b"attachment bytes")
        })),
        ("POST", "/gmail/v1/users/me/messages/msg-1/modify") => {
            assert_json_body(
                body,
                json!({"addLabelIds":["STARRED"],"removeLabelIds":["UNREAD","DRAFT"]}),
            );
            ok(json!({"id":"msg-1","labelIds":["INBOX","STARRED"]}))
        }
        ("POST", "/gmail/v1/users/me/messages/msg-mark-unread/modify") => {
            assert_json_body(
                body,
                json!({"addLabelIds":["UNREAD"],"removeLabelIds":[]}),
            );
            ok(json!({"id":"msg-mark-unread","labelIds":["INBOX","UNREAD"]}))
        }
        ("POST", "/gmail/v1/users/me/messages/batchModify") => {
            assert_json_body(
                body,
                json!({"ids":["msg-1"],"addLabelIds":["TRASH"],"removeLabelIds":["INBOX","SPAM"]}),
            );
            ok(json!({}))
        }
        ("GET", "/gmail/v1/users/me/history") => {
            let query = query.unwrap_or_default();
            assert!(query.contains("startHistoryId=hist-0"));
            ok(json!({
                "historyId": "hist-2",
                "history": [{
                    "id": "hist-2",
                    "messagesAdded": [{"message": {"id":"msg-2", "threadId":"thread-2"}}],
                    "labelsAdded": [
                        {"message": {"id":"msg-1", "threadId":"thread-1"}, "labelIds":["TRASH"]},
                        {"message": {"id":"msg-3", "threadId":"thread-3"}, "labelIds":["UNREAD"]}
                    ],
                    "labelsRemoved": [{"message": {"id":"msg-1", "threadId":"thread-1"}, "labelIds":["UNREAD"]}]
                }]
            }))
        }
        ("GET", "/gmail/v1/users/me/labels") => ok(json!({
            "labels": [
                {"id":"INBOX", "name":"INBOX", "type":"system"},
                {"id":"TRASH", "name":"TRASH", "type":"system"},
                {"id":"Label_1", "name":"Projects", "type":"user"}
            ]
        })),
        ("DELETE", "/gmail/v1/users/me/messages/msg-1") => ok(json!({})),
        _ => FakeResponse {
            status: StatusCode::NOT_FOUND,
            body: json!({"error":{"message": format!("no route for {method} {path}")}}),
        },
    }
}

fn ok(body: serde_json::Value) -> FakeResponse {
    FakeResponse {
        status: StatusCode::OK,
        body,
    }
}

fn assert_json_body(actual: &str, expected: serde_json::Value) {
    let actual: serde_json::Value = serde_json::from_str(actual).expect("json request body");
    assert_eq!(actual, expected);
}

async fn last_modify_body(state: &FakeGmailState) -> serde_json::Value {
    let requests = state.requests.lock().await;
    let request = requests
        .iter()
        .rev()
        .find(|request| request.path.ends_with("/modify"))
        .expect("modify request recorded");
    serde_json::from_str(&request.body).expect("json modify body")
}

async fn assert_recorded_requests_cover_every_gmail_rest_endpoint(state: &FakeGmailState) {
    let requests = state.requests.lock().await;
    let seen = requests
        .iter()
        .map(|request| (request.method.as_str(), request.path.as_str()))
        .collect::<HashSet<_>>();
    for expected in [
        ("GET", "/gmail/v1/users/me/messages"),
        ("GET", "/gmail/v1/users/me/messages/msg-1"),
        ("GET", "/gmail/v1/users/me/messages/msg-1/attachments/att-1"),
        ("POST", "/gmail/v1/users/me/messages/msg-1/modify"),
        ("POST", "/gmail/v1/users/me/messages/batchModify"),
        ("GET", "/gmail/v1/users/me/history"),
        ("GET", "/gmail/v1/users/me/labels"),
        ("DELETE", "/gmail/v1/users/me/messages/msg-1"),
    ] {
        assert!(
            seen.contains(&expected),
            "missing Gmail REST request {expected:?}; saw {requests:?}"
        );
    }
}
