use bytes::Bytes;
use hail_backend::conformance::{
    MailBackendConformance, PrincipalExpectation, run_mail_backend_conformance,
};
use hail_backend::{
    BackendMsgId, BlobRef, Change, Envelope, Keyword, Mailbox, MailboxRole, Principal, RawMessage,
    SubmissionId, SyncCursor,
};
use hail_jmap::{JMAP_BACKEND_CAPABILITIES, JmapBackend, login_basic};
use secrecy::SecretString;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const ACCOUNT_ID: &str = "account-test";
const MANAGEMENT_ACCOUNT_ID: &str = ACCOUNT_ID;

#[derive(Clone, Debug)]
struct RequestRecord {
    method: String,
    path: String,
    body: Value,
}

#[derive(Debug, Default)]
struct FakeJmapState {
    requests: tokio::sync::Mutex<Vec<RequestRecord>>,
    uploads: tokio::sync::Mutex<Vec<Vec<u8>>>,
}

#[tokio::test]
async fn jmap_backend_satisfies_shared_mail_backend_conformance() {
    let (base_url, state) = fake_jmap_server().await;
    let session = login_basic(
        &base_url,
        "user@example.org",
        SecretString::from("test-password"),
    )
    .await
    .expect("jmap session");
    let management = hail_jmap::management::ManagementSession::connect(
        &base_url,
        SecretString::from("management-token"),
    )
    .await
    .expect("management session");
    let backend = JmapBackend::with_management(session, management);
    let fixture = jmap_fixture();

    run_mail_backend_conformance(&backend, &fixture).await;

    assert_recorded_requests_cover_every_jmap_operation(&state, &fixture).await;
}

fn jmap_fixture() -> MailBackendConformance {
    let message_id = BackendMsgId::new("msg-1");
    let blob_ref = BlobRef::new("blob-att-1");
    MailBackendConformance {
        expected_capabilities: JMAP_BACKEND_CAPABILITIES,
        listed_message_ids: vec![message_id.clone()],
        listed_next_cursor: None,
        message_id: message_id.clone(),
        expected_message: RawMessage {
            id: message_id.clone(),
            thread_id: Some("thread-1".to_string()),
            rfc822: Bytes::from_static(
                b"From: sender@example.org\r\nTo: user@example.org\r\nSubject: JMAP fixture\r\n\r\nHello from JMAP",
            ),
            keywords: vec![Keyword::new("$seen"), Keyword::new("$flagged")],
            envelope: Some(Envelope {
                mail_from: "sender@example.org".to_string(),
                rcpt_to: vec!["user@example.org".to_string()],
            }),
            received_at_epoch_secs: Some(1_700_000_000),
            size_bytes: Some(86),
            blob_refs: vec![BlobRef::new("blob-msg-1"), BlobRef::new("blob-att-1")],
            attachments: vec![hail_backend::AttachmentMeta {
                filename: "fixture.txt".to_string(),
                mime_type: "text/plain".to_string(),
                size_bytes: 16,
                blob_ref: Some(BlobRef::new("blob-att-1")),
                inline: false,
                content_id: None,
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
        send_envelope: Envelope {
            mail_from: "sender@example.org".to_string(),
            rcpt_to: vec!["user@example.org".to_string()],
        },
        expected_submission_id: SubmissionId::new("submission-1"),
        poll_cursor: SyncCursor::new("state-0"),
        expected_changes: vec![
            Change::MessageCreated {
                id: BackendMsgId::new("msg-2"),
                raw_ref: None,
            },
            Change::MessageUpdated {
                id: BackendMsgId::new("msg-1"),
                keywords: Some(vec![Keyword::new("$flagged"), Keyword::new("$seen")]),
                keywords_added: Vec::new(),
                keywords_removed: Vec::new(),
            },
            Change::MessageDeleted {
                id: BackendMsgId::new("msg-deleted"),
            },
        ],
        expected_next_cursor: SyncCursor::new("state-1"),
        expected_mailboxes: vec![
            Mailbox {
                id: "inbox-id".to_string(),
                name: "Inbox".to_string(),
                role: MailboxRole::Inbox,
                parent_id: None,
            },
            Mailbox {
                id: "sent-id".to_string(),
                name: "Sent".to_string(),
                role: MailboxRole::Sent,
                parent_id: None,
            },
            Mailbox {
                id: "trash-id".to_string(),
                name: "Trash".to_string(),
                role: MailboxRole::Trash,
                parent_id: None,
            },
            Mailbox {
                id: "projects-id".to_string(),
                name: "Projects".to_string(),
                role: MailboxRole::Custom,
                parent_id: None,
            },
        ],
        principals: PrincipalExpectation::Supported(vec![Principal {
            id: "principal-1".to_string(),
            email: "user@example.org".to_string(),
            display_name: Some("Example User".to_string()),
        }]),
    }
}

async fn fake_jmap_server() -> (String, Arc<FakeJmapState>) {
    let state = Arc::new(FakeJmapState::default());
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
    (format!("http://{addr}"), state)
}

async fn handle_connection(mut stream: TcpStream, state: Arc<FakeJmapState>) {
    let response = match read_http_request(&mut stream).await {
        Ok(Some(request)) => route(request, state).await,
        Ok(None) => return,
        Err(error) => FakeResponse::json(
            400,
            json!({"type":"urn:ietf:params:jmap:error:badRequest","detail": error}),
        ),
    };
    let body = response.body;
    let headers = format!(
        "HTTP/1.1 {} OK\r\ncontent-type: {}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        response.status,
        response.content_type,
        body.len()
    );
    let _ = stream.write_all(headers.as_bytes()).await;
    let _ = stream.write_all(&body).await;
    let _ = stream.flush().await;
    let _ = stream.shutdown().await;
}

struct HttpRequest {
    method: String,
    path: String,
    host: String,
    body: Vec<u8>,
}

async fn read_http_request(stream: &mut TcpStream) -> Result<Option<HttpRequest>, String> {
    let mut buffer = Vec::new();
    let header_end = loop {
        let mut chunk = [0_u8; 1024];
        let read = stream
            .read(&mut chunk)
            .await
            .map_err(|error| format!("read request: {error}"))?;
        if read == 0 {
            if buffer.is_empty() {
                return Ok(None);
            }
            return Err("connection closed before headers completed".to_string());
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(header_end) = find_header_end(&buffer) {
            break header_end;
        }
    };

    let headers = String::from_utf8_lossy(&buffer[..header_end]);
    let mut lines = headers.lines();
    let request_line = lines.next().ok_or("missing request line")?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().ok_or("missing method")?.to_string();
    let target = parts.next().ok_or("missing target")?;
    let path = target
        .split_once('?')
        .map_or(target, |(path, _)| path)
        .to_string();
    let mut host = String::new();
    let mut content_length = 0;
    for (name, value) in lines.filter_map(|line| line.split_once(':')) {
        if name.eq_ignore_ascii_case("host") {
            host = value.trim().to_string();
        }
        if name.eq_ignore_ascii_case("content-length") {
            content_length = value.trim().parse::<usize>().unwrap_or(0);
        }
    }

    let body_start = header_end + 4;
    while buffer.len().saturating_sub(body_start) < content_length {
        let mut chunk = [0_u8; 1024];
        let read = stream
            .read(&mut chunk)
            .await
            .map_err(|error| format!("read request body: {error}"))?;
        if read == 0 {
            return Err("connection closed before body completed".to_string());
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    let body = buffer[body_start..body_start + content_length].to_vec();
    Ok(Some(HttpRequest {
        method,
        path,
        host,
        body,
    }))
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
    fn json(status: u16, body: Value) -> Self {
        Self {
            status,
            content_type: "application/json",
            body: body.to_string().into_bytes(),
        }
    }

    fn bytes(content_type: &'static str, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status: 200,
            content_type,
            body: body.into(),
        }
    }
}

async fn route(request: HttpRequest, state: Arc<FakeJmapState>) -> FakeResponse {
    let json_body = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
    state.requests.lock().await.push(RequestRecord {
        method: request.method.clone(),
        path: request.path.clone(),
        body: json_body.clone(),
    });

    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/.well-known/jmap") => {
            FakeResponse::json(200, session_document(&format!("http://{}", request.host)))
        }
        ("GET", path) if path.starts_with("/download/") => FakeResponse::bytes(
            "application/octet-stream",
            if path.contains("blob-att-1") {
                b"attachment bytes".to_vec()
            } else {
                b"From: sender@example.org\r\nTo: user@example.org\r\nSubject: JMAP fixture\r\n\r\nHello from JMAP".to_vec()
            },
        ),
        ("POST", path) if path.starts_with("/upload/") => {
            state.uploads.lock().await.push(request.body);
            FakeResponse::json(
                200,
                json!({"accountId": ACCOUNT_ID, "blobId": "uploaded-blob-1", "type": "application/octet-stream", "size": 82}),
            )
        }
        ("POST", "/jmap/") => FakeResponse::json(200, jmap_response(&json_body)),
        _ => FakeResponse::json(
            404,
            json!({"error": format!("no route for {} {}", request.method, request.path)}),
        ),
    }
}

fn session_document(base: &str) -> Value {
    json!({
        "capabilities": {
            "urn:ietf:params:jmap:core": {
                "maxSizeUpload": 50000000,
                "maxConcurrentUpload": 4,
                "maxSizeRequest": 50000000,
                "maxConcurrentRequests": 4,
                "maxCallsInRequest": 16,
                "maxObjectsInGet": 500,
                "maxObjectsInSet": 500,
                "collationAlgorithms": ["i;unicode-casemap"]
            },
            "urn:ietf:params:jmap:mail": {},
            "urn:ietf:params:jmap:submission": {},
            "urn:ietf:params:jmap:websocket": {"url": format!("{base}/ws"), "supportsPush": true},
            "urn:stalwart:jmap": {}
        },
        "accounts": {
            ACCOUNT_ID: {
                "name": "user@example.org",
                "isPersonal": true,
                "isReadOnly": false,
                "accountCapabilities": {"urn:ietf:params:jmap:mail": {}, "urn:ietf:params:jmap:submission": {}}
            }
        },
        "primaryAccounts": {
            "urn:ietf:params:jmap:mail": ACCOUNT_ID,
            "urn:ietf:params:jmap:submission": ACCOUNT_ID
        },
        "username": "user@example.org",
        "apiUrl": format!("{base}/jmap/"),
        "downloadUrl": format!("{base}/download/{{accountId}}/{{blobId}}/{{name}}?accept={{type}}"),
        "uploadUrl": format!("{base}/upload/{{accountId}}/"),
        "eventSourceUrl": format!("{base}/eventsource/?types={{types}}&closeafter={{closeafter}}&ping={{ping}}"),
        "state": "session-state-1"
    })
}

fn jmap_response(request: &Value) -> Value {
    let responses = request
        .get("methodCalls")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_array)
        .map(|call| {
            let method = call.first().and_then(Value::as_str).unwrap_or("error");
            let arguments = call.get(1).unwrap_or(&Value::Null);
            let tag = call.get(2).and_then(Value::as_str).unwrap_or("s0");
            json!([method, method_payload(method, arguments), tag])
        })
        .collect::<Vec<_>>();
    json!({"methodResponses": responses, "sessionState": "session-state-1"})
}

fn method_payload(method: &str, arguments: &Value) -> Value {
    match method {
        "Email/query" => {
            json!({"accountId": account_id(arguments), "queryState": "query-state-1", "canCalculateChanges": true, "position": 0, "ids": ["msg-1"], "total": 1, "limit": arguments.get("limit").cloned().unwrap_or(Value::Null)})
        }
        "Email/get" => {
            json!({"accountId": account_id(arguments), "state": "email-state-1", "list": email_get_list(arguments), "notFound": []})
        }
        "Email/set" => email_set_payload(arguments),
        "Email/changes" => {
            json!({"accountId": account_id(arguments), "oldState": arguments.get("sinceState").cloned().unwrap_or(Value::Null), "newState": "state-1", "hasMoreChanges": false, "created": ["msg-2"], "updated": ["msg-1"], "destroyed": ["msg-deleted"]})
        }
        "Mailbox/get" => {
            json!({"accountId": account_id(arguments), "state": "mailbox-state-1", "list": mailboxes(), "notFound": []})
        }
        "Email/import" => {
            json!({"accountId": account_id(arguments), "oldState": "email-state-1", "newState": "email-state-2", "created": {"i0": {"id": "sent-email-1", "blobId": "uploaded-blob-1", "threadId": "thread-sent", "keywords": {"$seen": true}, "size": 82}}})
        }
        "Identity/get" => {
            json!({"accountId": account_id(arguments), "state": "identity-state-1", "list": [{"id": "identity-1", "email": "sender@example.org", "name": "Sender"}], "notFound": []})
        }
        "EmailSubmission/set" => {
            json!({"accountId": account_id(arguments), "oldState": "submission-state-1", "newState": "submission-state-2", "created": {"c0": {"id": "submission-1", "emailId": "sent-email-1", "identityId": "identity-1", "undoStatus": "pending"}}})
        }
        "Principal/query" => {
            json!({"accountId": account_id(arguments), "queryState": "principal-query-state-1", "canCalculateChanges": true, "position": 0, "ids": ["principal-1"], "total": 1})
        }
        "Principal/get" => {
            json!({"accountId": account_id(arguments), "state": "principal-state-1", "list": [{"id": "principal-1", "name": "user@example.org", "type": "individual", "description": "Example User", "emails": ["user@example.org"]}], "notFound": []})
        }
        _ => json!({"type": "unknownMethod", "description": format!("unhandled {method}")}),
    }
}

fn account_id(arguments: &Value) -> Value {
    arguments
        .get("accountId")
        .cloned()
        .unwrap_or_else(|| json!(ACCOUNT_ID))
}

fn email_get_list(arguments: &Value) -> Value {
    let ids = arguments
        .get("ids")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_else(|| vec![json!("msg-1")]);
    Value::Array(
        ids.into_iter()
            .filter_map(|id| id.as_str().map(email_object))
            .collect(),
    )
}

fn email_object(id: &str) -> Value {
    match id {
        "msg-1" => {
            json!({"id": "msg-1", "blobId": "blob-msg-1", "threadId": "thread-1", "keywords": {"$seen": true, "$flagged": true}, "size": 86, "receivedAt": "2023-11-14T22:13:20Z", "from": [{"email": "sender@example.org"}], "to": [{"email": "user@example.org"}], "subject": "JMAP fixture", "attachments": [{"partId": "2", "blobId": "blob-att-1", "size": 16, "name": "fixture.txt", "type": "text/plain", "disposition": "attachment"}]})
        }
        "msg-2" => {
            json!({"id": "msg-2", "blobId": "blob-msg-2", "threadId": "thread-2", "keywords": {}, "size": 10})
        }
        other => {
            json!({"id": other, "blobId": format!("blob-{other}"), "keywords": {"$seen": true}, "size": 10})
        }
    }
}

fn email_set_payload(arguments: &Value) -> Value {
    let updated = arguments
        .get("update")
        .and_then(Value::as_object)
        .map(|updates| {
            updates
                .keys()
                .map(|id| (id.clone(), Value::Null))
                .collect::<serde_json::Map<_, _>>()
        })
        .unwrap_or_default();
    let destroyed = arguments
        .get("destroy")
        .cloned()
        .unwrap_or_else(|| json!([]));
    json!({"accountId": account_id(arguments), "oldState": "email-state-1", "newState": "email-state-2", "updated": updated, "destroyed": destroyed})
}

fn mailboxes() -> Value {
    json!([
        {"id": "inbox-id", "name": "Inbox", "role": "inbox", "parentId": null},
        {"id": "sent-id", "name": "Sent", "role": "sent", "parentId": null},
        {"id": "trash-id", "name": "Trash", "role": "trash", "parentId": null},
        {"id": "projects-id", "name": "Projects", "role": null, "parentId": null}
    ])
}

async fn assert_recorded_requests_cover_every_jmap_operation(
    state: &FakeJmapState,
    fixture: &MailBackendConformance,
) {
    let uploads = state.uploads.lock().await;
    assert_eq!(uploads.as_slice(), &[fixture.send_rfc822.clone()]);
    drop(uploads);

    let requests = state.requests.lock().await;
    let seen_paths = requests
        .iter()
        .map(|request| (request.method.as_str(), request.path.as_str()))
        .collect::<HashSet<_>>();
    for expected in [
        ("GET", "/.well-known/jmap"),
        ("POST", "/jmap/"),
        ("GET", "/download/account-test/blob-msg-1/none"),
        ("GET", "/download/account-test/blob-att-1/none"),
        ("POST", "/upload/account-test/"),
    ] {
        assert!(
            seen_paths.contains(&expected),
            "missing HTTP request {expected:?}; saw {requests:?}"
        );
    }

    let calls = requests
        .iter()
        .flat_map(|request| method_calls(&request.body))
        .collect::<Vec<_>>();
    for expected in [
        "Email/query",
        "Email/get",
        "Email/set",
        "Email/changes",
        "Mailbox/get",
        "Email/import",
        "Identity/get",
        "EmailSubmission/set",
        "Principal/query",
        "Principal/get",
    ] {
        assert!(
            calls.iter().any(|(method, _)| *method == expected),
            "missing JMAP method {expected}; saw {calls:?}"
        );
    }

    let email_query = calls
        .iter()
        .find(|(method, _)| *method == "Email/query")
        .expect("Email/query request")
        .1;
    assert_eq!(email_query.get("accountId"), Some(&json!(ACCOUNT_ID)));
    assert_eq!(email_query.get("limit"), Some(&json!(11)));

    let principal_query = calls
        .iter()
        .find(|(method, _)| *method == "Principal/query")
        .expect("Principal/query request")
        .1;
    assert_eq!(
        principal_query.get("accountId"),
        Some(&json!(MANAGEMENT_ACCOUNT_ID))
    );
}

fn method_calls(body: &Value) -> Vec<(&str, &Value)> {
    body.get("methodCalls")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|call| {
            let parts = call.as_array()?;
            Some((parts.first()?.as_str()?, parts.get(1)?))
        })
        .collect()
}
