use bytes::Bytes;
use hail_backend::conformance::{
    MailBackendConformance, PrincipalExpectation, run_mail_backend_conformance,
};
use hail_backend::{
    BackendMsgId, BlobRef, Capabilities, Change, Envelope, Keyword, Mailbox, MailboxRole,
    Principal, RawMessage, SubmissionId, SyncCursor,
};
use hail_jmap::{JmapBackend, login_basic};
use secrecy::SecretString;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const JMAP_CAPABILITIES: Capabilities = Capabilities {
    supports_initial_import: false,
    supports_eventsource: true,
    supports_principals_admin: true,
    supports_send: true,
    native_threading: true,
    max_attachment_size: u64::MAX,
    label_path_separator: '/',
};

#[derive(Clone, Debug)]
struct RequestRecord {
    method: String,
    path: String,
    body: Option<Value>,
}

#[derive(Debug, Default)]
struct FakeJmapState {
    requests: tokio::sync::Mutex<Vec<RequestRecord>>,
}

#[tokio::test]
async fn jmap_backend_satisfies_shared_mail_backend_conformance() {
    let (base_url, state) = fake_jmap_server().await;
    let session = login_basic(
        &base_url,
        "user@example.org",
        SecretString::from("password"),
    )
    .await
    .expect("jmap session");
    assert_eq!(session.account_id(), "account-test");
    let management = hail_jmap::management::ManagementSession::connect(
        &base_url,
        SecretString::from("management-token"),
    )
    .await
    .expect("management session");
    let backend = JmapBackend::with_management(session, management);
    let fixture = jmap_fixture();

    run_mail_backend_conformance(&backend, &fixture).await;

    assert_recorded_requests_cover_every_jmap_endpoint(&state).await;
}

fn jmap_fixture() -> MailBackendConformance {
    let message_id = BackendMsgId::new("email-1");
    let blob_ref = BlobRef::new("blob-attachment-1");
    MailBackendConformance {
        expected_capabilities: JMAP_CAPABILITIES,
        listed_message_ids: vec![message_id.clone()],
        listed_next_cursor: None,
        message_id: message_id.clone(),
        expected_message: RawMessage {
            id: message_id.clone(),
            thread_id: Some("thread-1".to_string()),
            rfc822: Bytes::from_static(JMAP_RFC822),
            keywords: vec![Keyword::new("$flagged"), Keyword::new("$seen")],
            envelope: Some(Envelope { mail_from: "sender@example.org".to_string(), rcpt_to: vec!["user@example.org".to_string()] }),
            received_at_epoch_secs: Some(1_779_538_500),
            size_bytes: Some(1234),
            blob_refs: vec![BlobRef::new("blob-email-1"), BlobRef::new("blob-attachment-1")],
            attachments: vec![hail_backend::AttachmentMeta {
                filename: "report.pdf".to_string(), mime_type: "application/pdf".to_string(), size_bytes: 42,
                blob_ref: Some(BlobRef::new("blob-attachment-1")), inline: false, content_id: None,
            }],
            metadata: BTreeMap::from([
                ("subject".to_string(), "JMAP fixture".to_string()),
                ("from".to_string(), "sender@example.org".to_string()),
                ("to".to_string(), "user@example.org".to_string()),
            ]),
        },
        blob_ref,
        expected_blob: Bytes::from_static(b"attachment bytes"),
        keyword_additions: vec![Keyword::new("$flagged")],
        keyword_removals: vec![Keyword::new("$seen")],
        move_role: MailboxRole::Trash,
        send_rfc822: b"From: sender@example.org\r\nTo: user@example.org\r\nSubject: conformance send\r\n\r\nBody".to_vec(),
        send_envelope: Envelope { mail_from: "sender@example.org".to_string(), rcpt_to: vec!["user@example.org".to_string()] },
        expected_submission_id: SubmissionId::new("submission-1"),
        poll_cursor: SyncCursor::new("state-0"),
        expected_changes: vec![
            Change::MessageCreated { id: BackendMsgId::new("email-2"), raw_ref: None },
            Change::MessageUpdated { id: BackendMsgId::new("email-1"), keywords: Some(vec![Keyword::new("$seen"), Keyword::new("$flagged")]), keywords_added: Vec::new(), keywords_removed: Vec::new() },
            Change::MessageDeleted { id: BackendMsgId::new("email-deleted") },
        ],
        expected_next_cursor: SyncCursor::new("state-2"),
        expected_mailboxes: fixture_mailboxes(),
        principals: PrincipalExpectation::Supported(vec![Principal { id: "principal-1".to_string(), email: "alice@example.org".to_string(), display_name: Some("Alice Example".to_string()) }]),
    }
}

const JMAP_RFC822: &[u8] = b"From: Sender <sender@example.org>\r\nTo: User <user@example.org>\r\nSubject: JMAP fixture\r\n\r\nHello from JMAP";

fn fixture_mailboxes() -> Vec<Mailbox> {
    vec![
        Mailbox {
            id: "inbox".to_string(),
            name: "Inbox".to_string(),
            role: MailboxRole::Inbox,
            parent_id: None,
        },
        Mailbox {
            id: "trash".to_string(),
            name: "Trash".to_string(),
            role: MailboxRole::Trash,
            parent_id: None,
        },
        Mailbox {
            id: "sent".to_string(),
            name: "Sent".to_string(),
            role: MailboxRole::Sent,
            parent_id: None,
        },
        Mailbox {
            id: "projects".to_string(),
            name: "Projects".to_string(),
            role: MailboxRole::Custom,
            parent_id: Some("inbox".to_string()),
        },
    ]
}

async fn fake_jmap_server() -> (String, Arc<FakeJmapState>) {
    let state = Arc::new(FakeJmapState::default());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let base_url = format!("http://{addr}");
    let server_state = Arc::clone(&state);
    let server_base_url = base_url.clone();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let state = Arc::clone(&server_state);
            let base_url = server_base_url.clone();
            tokio::spawn(async move {
                handle_connection(stream, state, base_url).await;
            });
        }
    });
    (base_url, state)
}

async fn handle_connection(mut stream: TcpStream, state: Arc<FakeJmapState>, base_url: String) {
    let mut buffer = Vec::with_capacity(32 * 1024);
    let mut temp = [0_u8; 4096];
    let header_end;
    loop {
        let read = stream.read(&mut temp).await.expect("read request");
        assert_ne!(read, 0, "connection closed before request headers");
        buffer.extend_from_slice(&temp[..read]);
        if let Some(end) = find_header_end(&buffer) {
            header_end = end;
            break;
        }
    }
    let headers = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().expect("content-length"))
        })
        .unwrap_or(0);
    let body_start = header_end + 4;
    while buffer.len() < body_start + content_length {
        let read = stream.read(&mut temp).await.expect("read request body");
        assert_ne!(read, 0, "connection closed before request body");
        buffer.extend_from_slice(&temp[..read]);
    }
    let body = String::from_utf8_lossy(&buffer[body_start..body_start + content_length]);
    let mut lines = headers.lines();
    let request_line = lines.next().expect("request line");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().expect("method").to_string();
    let target = parts.next().expect("target");
    let path = target
        .split_once('?')
        .map_or(target, |(path, _)| path)
        .to_string();
    let content_type = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-type")
                .then(|| value.trim().to_ascii_lowercase())
        })
        .unwrap_or_default();
    let body_json = if body.trim().is_empty() || !content_type.starts_with("application/json") {
        None
    } else {
        Some(serde_json::from_str(&body).expect("json request body"))
    };
    state.requests.lock().await.push(RequestRecord {
        method: method.clone(),
        path: path.clone(),
        body: body_json.clone(),
    });

    let response = route(&base_url, &method, &path, body_json.as_ref());
    stream
        .write_all(&response.to_http())
        .await
        .expect("write response");
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

struct FakeResponse {
    status: u16,
    content_type: &'static str,
    body: Vec<u8>,
}

impl FakeResponse {
    fn json(body: Value) -> Self {
        Self {
            status: 200,
            content_type: "application/json",
            body: body.to_string().into_bytes(),
        }
    }
    fn bytes(body: &'static [u8]) -> Self {
        Self {
            status: 200,
            content_type: "application/octet-stream",
            body: body.to_vec(),
        }
    }
    fn not_found(method: &str, path: &str) -> Self {
        Self {
            status: 404,
            content_type: "application/json",
            body: json!({"error": format!("no route for {method} {path}")})
                .to_string()
                .into_bytes(),
        }
    }
    fn to_http(&self) -> Vec<u8> {
        let reason = if self.status == 200 {
            "OK"
        } else {
            "Not Found"
        };
        let mut response = format!(
            "HTTP/1.1 {} {}\r\ncontent-type: {}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            self.status,
            reason,
            self.content_type,
            self.body.len()
        )
        .into_bytes();
        response.extend_from_slice(&self.body);
        response
    }
}

fn route(base_url: &str, method: &str, path: &str, body: Option<&Value>) -> FakeResponse {
    match (method, path) {
        ("GET", "/.well-known/jmap") => FakeResponse::json(session_response(base_url)),
        ("GET", path) if path.starts_with("/download/account-test/blob-email-1/") => {
            FakeResponse::bytes(JMAP_RFC822)
        }
        ("GET", path) if path.starts_with("/download/account-test/blob-attachment-1/") => {
            FakeResponse::bytes(b"attachment bytes")
        }
        ("POST", "/upload/account-test/") => FakeResponse::json(
            json!({"accountId":"account-test","blobId":"blob-upload-1","type":"application/octet-stream","size":78}),
        ),
        ("POST", "/jmap/") => FakeResponse::json(jmap_response(body.expect("jmap body"))),
        _ => FakeResponse::not_found(method, path),
    }
}

fn session_response(base_url: &str) -> Value {
    json!({
        "capabilities": {
            "urn:ietf:params:jmap:core": {"maxSizeUpload": 50_000_000, "maxConcurrentUpload": 4, "maxSizeRequest": 10_000_000, "maxConcurrentRequests": 4, "maxCallsInRequest": 16, "maxObjectsInGet": 500, "maxObjectsInSet": 500, "collationAlgorithms": ["i;unicode-casemap"]},
            "urn:ietf:params:jmap:mail": {"maxMailboxesPerEmail": 16, "maxMailboxDepth": 10, "maxSizeMailboxName": 255, "maxSizeAttachmentsPerEmail": 50_000_000, "emailQuerySortOptions": ["receivedAt"], "mayCreateTopLevelMailbox": true},
            "urn:stalwart:jmap": {}
        },
        "accounts": {
            "account-test": {"name": "Test Account", "isPersonal": true, "isReadOnly": false, "accountCapabilities": {"urn:ietf:params:jmap:mail": {}}}
        },
        "primaryAccounts": {"urn:ietf:params:jmap:mail": "account-test", "urn:stalwart:jmap": "mgmt-account"},
        "username": "user@example.org",
        "apiUrl": format!("{base_url}/jmap/"),
        "downloadUrl": format!("{base_url}/download/{{accountId}}/{{blobId}}/{{name}}?accept={{type}}"),
        "uploadUrl": format!("{base_url}/upload/{{accountId}}/"),
        "eventSourceUrl": format!("{base_url}/eventsource/?types={{types}}&closeafter={{closeafter}}&ping={{ping}}"),
        "state": "fake-state"
    })
}

fn jmap_response(request: &Value) -> Value {
    let responses = request["methodCalls"]
        .as_array()
        .expect("methodCalls array")
        .iter()
        .map(method_response)
        .collect::<Vec<_>>();
    json!({"sessionState": "fake-state", "methodResponses": responses})
}

fn method_response(call: &Value) -> Value {
    let method = call[0].as_str().expect("method name");
    let arguments = &call[1];
    let tag = call[2].as_str().unwrap_or("s0");
    match method {
        "Email/query" => {
            assert_eq!(arguments["accountId"], "account-test");
            assert_eq!(arguments["limit"], 11);
            json!([method, {"accountId":"account-test","queryState":"query-state-1","canCalculateChanges":false,"position":0,"total":1,"ids":["email-1"]}, tag])
        }
        "Email/get" => {
            assert_eq!(arguments["accountId"], "account-test");
            let ids = string_array(&arguments["ids"]);
            let list = if ids == ["email-1"] {
                vec![email_one_json()]
            } else {
                vec![email_keywords_json()]
            };
            json!([method, {"accountId":"account-test","state":"email-state-1","list":list,"notFound":[]}, tag])
        }
        "Email/set" => {
            assert_eq!(arguments["accountId"], "account-test");
            if let Some(update) = arguments.get("update").and_then(Value::as_object) {
                assert!(
                    update.contains_key("email-1"),
                    "unexpected Email/set update {update:?}"
                );
                json!([method, {"accountId":"account-test","oldState":"email-state-1","newState":"email-state-2","updated":{"email-1":null}}, tag])
            } else {
                assert_eq!(string_array(&arguments["destroy"]), ["email-1"]);
                json!([method, {"accountId":"account-test","oldState":"email-state-2","newState":"email-state-3","destroyed":["email-1"]}, tag])
            }
        }
        "Mailbox/get" => {
            assert_eq!(arguments["accountId"], "account-test");
            json!([method, {"accountId":"account-test","state":"mailbox-state-1","list":mailboxes_json(),"notFound":[]}, tag])
        }
        "Email/changes" => {
            assert_eq!(arguments["accountId"], "account-test");
            assert_eq!(arguments["sinceState"], "state-0");
            json!([method, {"accountId":"account-test","oldState":"state-0","newState":"state-2","hasMoreChanges":false,"created":["email-2"],"updated":["email-1"],"destroyed":["email-deleted"]}, tag])
        }
        "Email/import" => {
            assert_eq!(arguments["accountId"], "account-test");
            json!([method, {"accountId":"account-test","oldState":"email-state-2","newState":"email-state-3","created":{"i0":{"id":"sent-email-1","blobId":"blob-upload-1","threadId":"thread-sent"},"c0":{"id":"sent-email-1","blobId":"blob-upload-1","threadId":"thread-sent"}}}, tag])
        }
        "Identity/get" => {
            assert_eq!(arguments["accountId"], "account-test");
            json!([method, {"accountId":"account-test","state":"identity-state-1","list":[{"id":"identity-1","email":"sender@example.org"}],"notFound":[]}, tag])
        }
        "EmailSubmission/set" => {
            assert_eq!(arguments["accountId"], "account-test");
            json!([method, {"accountId":"account-test","oldState":"submission-state-0","newState":"submission-state-1","created":{"i0":{"id":"submission-1","emailId":"sent-email-1","identityId":"identity-1"},"c0":{"id":"submission-1","emailId":"sent-email-1","identityId":"identity-1"}}}, tag])
        }
        "Principal/query" => {
            assert_eq!(arguments["accountId"], "mgmt-account");
            assert_eq!(arguments["filter"], json!({"type":"individual"}));
            json!([method, {"accountId":"mgmt-account","queryState":"principal-query-state","canCalculateChanges":false,"position":0,"total":1,"ids":["principal-1"]}, tag])
        }
        "Principal/get" => {
            assert_eq!(arguments["accountId"], "mgmt-account");
            json!([method, {"accountId":"mgmt-account","state":"principal-state","list":[{"id":"principal-1","name":"alice","type":"individual","description":"Alice Example","emails":["alice@example.org"]}],"notFound":[]}, tag])
        }
        other => panic!("unexpected JMAP method {other} with arguments {arguments}"),
    }
}

fn email_one_json() -> Value {
    json!({
        "id":"email-1", "blobId":"blob-email-1", "threadId":"thread-1", "keywords":{"$seen":true,"$flagged":true},
        "receivedAt":"2026-05-23T12:15:00Z", "size":1234,
        "from":[{"name":"Sender","email":"sender@example.org"}],
        "to":[{"name":"User","email":"user@example.org"}], "cc":[], "bcc":[], "subject":"JMAP fixture",
        "attachments":[{"partId":"att-1","blobId":"blob-attachment-1","name":"report.pdf","type":"application/pdf","size":42,"disposition":"attachment"}]
    })
}

fn email_keywords_json() -> Value {
    json!({"id":"email-1", "keywords":{"$seen":true}})
}

fn mailboxes_json() -> Value {
    json!([
        {"id":"inbox","name":"Inbox","role":"inbox"},
        {"id":"trash","name":"Trash","role":"trash"},
        {"id":"sent","name":"Sent","role":"sent"},
        {"id":"projects","name":"Projects","role":null,"parentId":"inbox"}
    ])
}

fn string_array(value: &Value) -> Vec<String> {
    value
        .as_array()
        .expect("string array")
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

async fn assert_recorded_requests_cover_every_jmap_endpoint(state: &FakeJmapState) {
    let requests = state.requests.lock().await;
    let endpoints = requests
        .iter()
        .map(|request| (request.method.as_str(), request.path.as_str()))
        .collect::<HashSet<_>>();
    for expected in [
        ("GET", "/.well-known/jmap"),
        ("POST", "/jmap/"),
        ("GET", "/download/account-test/blob-email-1/none"),
        ("GET", "/download/account-test/blob-attachment-1/none"),
        ("POST", "/upload/account-test/"),
    ] {
        assert!(
            endpoints.contains(&expected),
            "missing JMAP endpoint {expected:?}; saw {requests:?}"
        );
    }

    let methods = requests
        .iter()
        .filter_map(|request| request.body.as_ref())
        .flat_map(|body| {
            body["methodCalls"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|call| call[0].as_str())
        })
        .collect::<HashSet<_>>();
    for expected in [
        "Email/query",
        "Email/get",
        "Email/set",
        "Email/changes",
        "Email/import",
        "EmailSubmission/set",
        "Identity/get",
        "Mailbox/get",
        "Principal/query",
        "Principal/get",
    ] {
        assert!(
            methods.contains(expected),
            "missing JMAP method {expected}; saw {methods:?}"
        );
    }
}
