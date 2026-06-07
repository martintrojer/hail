use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use bytes::Bytes;
use chrono::{Duration, Utc};
use hail_api::{
    middleware::auth::{CSRF_HEADER, require_auth},
    routes::screener::{Classification, ScreenerBackfill, ScreenerBackfillError, ScreenerDecision},
    state::AppState,
};
use hail_backend::{BackendMsgId, Envelope, Keyword, RawMessage};
use hail_blob_store::{BlobStore, FilesystemBlobStore};
use hail_cache::{CachePolicy, CachedMail};
use hail_core::{MailBackfill, MailCacheMode};
use hail_test::{fixture_state, json_body, seed_session};
use tower::ServiceExt;

async fn seed_rule(
    state: &AppState,
    user_id: i64,
    sender: &str,
    decision: &str,
    classify_as: Option<&str>,
    first_seen_at: chrono::DateTime<Utc>,
) {
    seed_rule_with_decided_at(
        state,
        user_id,
        sender,
        decision,
        classify_as,
        first_seen_at,
        None,
    )
    .await;
}

async fn seed_rule_with_decided_at(
    state: &AppState,
    user_id: i64,
    sender: &str,
    decision: &str,
    classify_as: Option<&str>,
    first_seen_at: chrono::DateTime<Utc>,
    decided_at: Option<chrono::DateTime<Utc>>,
) {
    sqlx::query("INSERT INTO screener_rules (user_id, sender_address, decision, classify_as, decided_at, first_seen_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)")
        .bind(user_id)
        .bind(sender)
        .bind(decision)
        .bind(classify_as)
        .bind(decided_at)
        .bind(first_seen_at)
        .execute(&state.db)
        .await
        .unwrap();
}

async fn set_latest_pending_received_at(
    state: &AppState,
    user_id: i64,
    sender: &str,
    latest_pending_received_at: chrono::DateTime<Utc>,
) {
    sqlx::query(
        "UPDATE screener_rules SET latest_pending_received_at = ?1 \
         WHERE user_id = ?2 AND sender_address = ?3",
    )
    .bind(latest_pending_received_at)
    .bind(user_id)
    .bind(sender)
    .execute(&state.db)
    .await
    .unwrap();
}

async fn seed_mail_account(state: &AppState, user_id: i64, email: &str) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO mail_accounts \
         (user_id, jmap_account_id, backend_kind, provider_kind, provider_account_id, provider_email, refresh_token_enc, sync_status, created_at, updated_at) \
         VALUES (?1, ?2, 'gmail', 'gmail', ?2, ?3, ?4, 'active', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z') RETURNING id",
    )
    .bind(user_id)
    .bind(format!("provider-{user_id}"))
    .bind(email)
    .bind(vec![7_u8; 32])
    .fetch_one(&state.db)
    .await
    .expect("insert mail account")
}

async fn seed_cached_message(
    state: &AppState,
    account_id: i64,
    backend_id: &str,
    from: &str,
    pinned: bool,
) {
    sqlx::query(
        "INSERT INTO messages \
         (account_id, backend_msg_id, thread_id, internal_date, from_addr, subject, preview, size_bytes, body_blob_id, body_text, inserted_at, accessed_at, pinned) \
         VALUES (?1, ?2, ?3, 1, ?4, 'subject', 'preview', 10, 'blob-id', 'body', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', ?5)",
    )
    .bind(account_id)
    .bind(backend_id)
    .bind(format!("thread-{backend_id}"))
    .bind(from)
    .bind(i64::from(pinned))
    .execute(&state.db)
    .await
    .expect("insert cached message");
}

async fn cached_pin(state: &AppState, backend_id: &str) -> i64 {
    sqlx::query_scalar("SELECT pinned FROM messages WHERE backend_msg_id = ?1")
        .bind(backend_id)
        .fetch_one(&state.db)
        .await
        .expect("cached pin")
}

fn app(state: AppState, backfill: Arc<FakeBackfill>) -> Router {
    let protected = hail_api::routes::screener::router_with_backfill(backfill).layer(
        axum::middleware::from_fn_with_state(state.clone(), require_auth),
    );
    Router::new().merge(protected).with_state(state)
}

async fn state_with_cache_mail(
    state: &AppState,
    user_id: i64,
    messages: Vec<RawMessage>,
) -> (AppState, TestMailBackend) {
    sqlx::query(
        "INSERT OR IGNORE INTO mail_accounts \
         (id, user_id, jmap_account_id, backend_kind, provider_kind, provider_account_id, provider_email, refresh_token_enc, sync_status, created_at, updated_at) \
         VALUES (1, ?1, 'acct', 'gmail', 'gmail', 'provider-acct', 'cache@example.test', ?2, 'active', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .bind(user_id)
    .bind(vec![7_u8; 32])
    .execute(&state.db)
    .await
    .expect("insert cache mail account");
    let tempdir = tempfile::tempdir().expect("create temp blob dir");
    let blob_root = tempdir.keep();
    let blobs = Arc::new(FilesystemBlobStore::new(blob_root)) as Arc<dyn BlobStore>;
    let backend = TestMailBackend::with_messages(messages);
    let state = AppState {
        db: state.db.clone(),
        config: state.config.clone(),
        server_key: state.server_key.clone(),
        auth_rate_limiter: state.auth_rate_limiter.clone(),
        mail: Arc::new(CachedMail::new(
            state.db.clone(),
            blobs,
            Box::new(backend.clone()),
            CachePolicy::new(
                MailCacheMode::Bounded,
                90,
                50_000,
                5 * 1024 * 1024,
                MailBackfill::Off,
            ),
        )),
        events: state.events.clone(),
    };
    (state, backend)
}

#[derive(Clone, Default)]
struct TestMailBackend {
    messages: Arc<HashMap<BackendMsgId, RawMessage>>,
    order: Arc<Vec<BackendMsgId>>,
    get_calls: Arc<Mutex<Vec<BackendMsgId>>>,
}

impl TestMailBackend {
    fn with_messages(messages: Vec<RawMessage>) -> Self {
        let order = messages
            .iter()
            .map(|message| message.id.clone())
            .collect::<Vec<_>>();
        let messages = messages
            .into_iter()
            .map(|message| (message.id.clone(), message))
            .collect::<HashMap<_, _>>();
        Self {
            messages: Arc::new(messages),
            order: Arc::new(order),
            get_calls: Arc::default(),
        }
    }

    fn get_calls(&self) -> Vec<String> {
        self.get_calls
            .lock()
            .expect("get calls lock")
            .iter()
            .map(|id| id.as_str().to_owned())
            .collect()
    }
}

#[async_trait]
impl hail_backend::MailBackend for TestMailBackend {
    fn capabilities(&self) -> &'static hail_backend::Capabilities {
        hail_api::test_support::FakeMailBackend::empty().capabilities()
    }

    async fn list_message_ids(
        &self,
        _query: &hail_backend::Query,
        page: &hail_backend::PageRequest,
    ) -> hail_backend::Result<hail_backend::Page<BackendMsgId>> {
        let limit = usize::try_from(page.limit).unwrap_or(usize::MAX);
        Ok(hail_backend::Page {
            items: self.order.iter().take(limit).cloned().collect(),
            next_cursor: None,
        })
    }

    async fn get_message(&self, id: &BackendMsgId) -> hail_backend::Result<RawMessage> {
        self.get_calls
            .lock()
            .expect("get calls lock")
            .push(id.clone());
        self.messages
            .get(id)
            .cloned()
            .ok_or_else(|| hail_backend::Error::NotFound {
                kind: "message",
                id: id.as_str().to_owned(),
            })
    }

    async fn fetch_blob(&self, id: &hail_backend::BlobRef) -> hail_backend::Result<Bytes> {
        Err(hail_backend::Error::NotFound {
            kind: "blob",
            id: id.as_str().to_owned(),
        })
    }

    async fn set_keywords(
        &self,
        _id: &BackendMsgId,
        _add: &[Keyword],
        _remove: &[Keyword],
    ) -> hail_backend::Result<()> {
        Ok(())
    }

    async fn move_to_role(
        &self,
        _id: &BackendMsgId,
        _role: hail_backend::MailboxRole,
    ) -> hail_backend::Result<()> {
        Ok(())
    }

    async fn delete_permanently(&self, _id: &BackendMsgId) -> hail_backend::Result<()> {
        Ok(())
    }

    async fn send(
        &self,
        _rfc822: &[u8],
        _envelope: &Envelope,
    ) -> hail_backend::Result<hail_backend::SubmissionId> {
        Ok(hail_backend::SubmissionId::new("fake-submission"))
    }

    async fn poll_changes(
        &self,
        cursor: &hail_backend::SyncCursor,
    ) -> hail_backend::Result<(Vec<hail_backend::Change>, hail_backend::SyncCursor)> {
        Ok((Vec::new(), cursor.clone()))
    }

    async fn watch_changes(
        &self,
    ) -> futures_util::stream::BoxStream<'static, hail_backend::Change> {
        Box::pin(futures_util::stream::empty())
    }

    async fn list_mailboxes(&self) -> hail_backend::Result<Vec<hail_backend::Mailbox>> {
        Ok(Vec::new())
    }

    async fn list_principals(&self) -> hail_backend::Result<Vec<hail_backend::Principal>> {
        Ok(Vec::new())
    }
}

fn raw_screener_message(
    id: &str,
    sender: &str,
    subject: &str,
    preview: &str,
    body: &str,
    content_type: &str,
    received_at_epoch_secs: i64,
) -> RawMessage {
    let mut metadata = BTreeMap::new();
    metadata.insert("subject".to_owned(), subject.to_owned());
    metadata.insert("preview".to_owned(), preview.to_owned());
    RawMessage {
        id: BackendMsgId::new(id),
        thread_id: Some(format!("thread-{id}")),
        rfc822: Bytes::from(format!(
            "From: {sender}\r\nTo: me@example.org\r\nSubject: {subject}\r\nMIME-Version: 1.0\r\nContent-Type: {content_type}; charset=utf-8\r\n\r\n{body}"
        )),
        keywords: vec![Keyword::new("$hail_screener")],
        envelope: Some(Envelope {
            mail_from: sender.to_owned(),
            rcpt_to: vec!["me@example.org".to_owned()],
        }),
        received_at_epoch_secs: Some(received_at_epoch_secs),
        size_bytes: Some(u64::try_from(body.len()).expect("body length fits u64")),
        blob_refs: Vec::new(),
        attachments: Vec::new(),
        metadata,
    }
}

async fn request(
    state: AppState,
    method: Method,
    uri: &str,
    sid: Option<&str>,
    csrf: bool,
    body: Option<&str>,
) -> axum::response::Response {
    request_with_backfill(
        state,
        Arc::new(FakeBackfill::default()),
        method,
        uri,
        sid,
        csrf,
        body,
    )
    .await
}

async fn request_with_backfill(
    state: AppState,
    backfill: Arc<FakeBackfill>,
    method: Method,
    uri: &str,
    sid: Option<&str>,
    csrf: bool,
    body: Option<&str>,
) -> axum::response::Response {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(sid) = sid {
        builder = builder.header(header::COOKIE, format!("hail_session={sid}"));
    }
    if csrf {
        builder = builder.header(CSRF_HEADER, "1");
    }
    if body.is_some() {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
    }
    app(state, backfill)
        .oneshot(
            builder
                .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
                .unwrap(),
        )
        .await
        .unwrap()
}

fn sender<'a>(senders: &'a [serde_json::Value], address: &str) -> &'a serde_json::Value {
    senders
        .iter()
        .find(|sender| sender["sender"] == address)
        .unwrap_or_else(|| panic!("sender {address} present"))
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct BackfillCall {
    user_id: i64,
    sender: String,
    decision: ScreenerDecision,
    classify_as: Option<Classification>,
}

#[derive(Default)]
struct FakeBackfill {
    calls: Mutex<Vec<BackfillCall>>,
    fail: bool,
}

impl FakeBackfill {
    fn failing() -> Self {
        Self {
            calls: Mutex::default(),
            fail: true,
        }
    }
}

#[async_trait]
impl ScreenerBackfill for FakeBackfill {
    async fn apply(
        &self,
        _state: &AppState,
        user: &hail_api::middleware::auth::AuthUser,
        sender: &str,
        decision: ScreenerDecision,
        classify_as: Option<Classification>,
    ) -> Result<(), ScreenerBackfillError> {
        self.calls.lock().unwrap().push(BackfillCall {
            user_id: user.id,
            sender: sender.to_string(),
            decision,
            classify_as,
        });
        if self.fail {
            return Err(ScreenerBackfillError("forced backfill failure".to_string()));
        }
        Ok(())
    }

    async fn apply_undo_deny(
        &self,
        _state: &AppState,
        user: &hail_api::middleware::auth::AuthUser,
        sender: &str,
        classify_as: Classification,
    ) -> Result<(), ScreenerBackfillError> {
        self.calls.lock().unwrap().push(BackfillCall {
            user_id: user.id,
            sender: sender.to_string(),
            decision: ScreenerDecision::Approve,
            classify_as: Some(classify_as),
        });
        if self.fail {
            return Err(ScreenerBackfillError("forced backfill failure".to_string()));
        }
        Ok(())
    }
}

#[tokio::test]
async fn screener_view_requires_auth() {
    let (state, _) = fixture_state().await;
    let resp = request(state, Method::GET, "/api/views/screener", None, false, None).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn screener_view_returns_only_current_user_pending_rows() {
    let (state, key) = fixture_state().await;
    let (alice_id, alice_sid) = seed_session(&state, &key, "alice@example.org").await;
    let (bob_id, _) = seed_session(&state, &key, "bob@example.org").await;
    let (state, _backend) = state_with_cache_mail(
        &state,
        alice_id,
        vec![
            raw_screener_message(
                "alice-email",
                "alice-pending@example.org",
                "Alice pending",
                "Alice private preview",
                "Alice private body",
                "text/plain",
                1_700_000_010,
            ),
            raw_screener_message(
                "bob-email",
                "bob-pending@example.org",
                "Bob pending",
                "Bob private preview",
                "Bob private body",
                "text/plain",
                1_700_000_020,
            ),
        ],
    )
    .await;
    let now = Utc::now();
    seed_rule(
        &state,
        alice_id,
        "alice-pending@example.org",
        "pending",
        None,
        now,
    )
    .await;
    seed_rule(
        &state,
        bob_id,
        "bob-pending@example.org",
        "pending",
        None,
        now,
    )
    .await;
    let resp = request(
        state,
        Method::GET,
        "/api/views/screener",
        Some(&alice_sid),
        false,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    let senders = json["senders"].as_array().unwrap();
    assert_eq!(senders.len(), 1);
    assert_eq!(senders[0]["sender"], "alice-pending@example.org");
    assert_eq!(senders[0]["message_count"], 1);
    assert_eq!(
        senders[0]["latest_preview"]["preview"],
        "Alice private preview"
    );
    assert_eq!(senders[0]["emails"].as_array().unwrap().len(), 1);
    assert_eq!(senders[0]["emails"][0]["email_id"], "alice-email");
    let rendered = serde_json::to_string(&json).expect("screener json serializes");
    assert!(!rendered.contains("bob-pending@example.org"));
    assert!(!rendered.contains("Bob private preview"));
    assert!(!rendered.contains("bob-email"));
    assert!(json["next_cursor"].is_null());
}

#[tokio::test]
async fn screener_view_omits_allowed_and_denied_rows() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let now = Utc::now();
    seed_rule(&state, user_id, "pending@example.org", "pending", None, now).await;
    seed_rule(
        &state,
        user_id,
        "allowed@example.org",
        "allow",
        Some("feed"),
        now,
    )
    .await;
    seed_rule(&state, user_id, "denied@example.org", "deny", None, now).await;
    let resp = request(
        state,
        Method::GET,
        "/api/views/screener",
        Some(&sid),
        false,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    let senders = json["senders"].as_array().unwrap();
    assert_eq!(senders.len(), 1);
    assert_eq!(senders[0]["sender"], "pending@example.org");
}

#[tokio::test]
async fn screener_view_sorts_newest_first() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let now = Utc::now();
    seed_rule(
        &state,
        user_id,
        "old@example.org",
        "pending",
        None,
        now - Duration::hours(2),
    )
    .await;
    seed_rule(&state, user_id, "new@example.org", "pending", None, now).await;
    seed_rule(
        &state,
        user_id,
        "middle@example.org",
        "pending",
        None,
        now - Duration::hours(1),
    )
    .await;
    let resp = request(
        state,
        Method::GET,
        "/api/views/screener",
        Some(&sid),
        false,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    let got: Vec<&str> = json["senders"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["sender"].as_str().unwrap())
        .collect();
    assert_eq!(
        got,
        vec!["new@example.org", "middle@example.org", "old@example.org"]
    );
    assert!(json["next_cursor"].is_null());
}

#[tokio::test]
async fn screener_view_paginates_pending_senders_with_cursor() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let now = Utc::now();
    let fixtures = [
        ("newest@example.org", now),
        ("same-a@example.org", now - Duration::minutes(1)),
        ("same-b@example.org", now - Duration::minutes(1)),
        ("same-c@example.org", now - Duration::minutes(1)),
        ("oldest@example.org", now - Duration::minutes(2)),
    ];
    for (sender, first_seen_at) in fixtures {
        seed_rule(&state, user_id, sender, "pending", None, first_seen_at).await;
    }

    let first = request(
        state.clone(),
        Method::GET,
        "/api/views/screener?limit=2",
        Some(&sid),
        false,
        None,
    )
    .await;
    assert_eq!(first.status(), StatusCode::OK);
    let first = json_body(first).await;
    let first_senders: Vec<&str> = first["senders"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["sender"].as_str().unwrap())
        .collect();
    assert_eq!(
        first_senders,
        vec!["newest@example.org", "same-a@example.org"]
    );
    let first_cursor = first["next_cursor"]
        .as_str()
        .expect("first page cursor")
        .to_owned();

    let second = request(
        state.clone(),
        Method::GET,
        &format!("/api/views/screener?limit=2&cursor={first_cursor}"),
        Some(&sid),
        false,
        None,
    )
    .await;
    assert_eq!(second.status(), StatusCode::OK);
    let second = json_body(second).await;
    let second_senders: Vec<&str> = second["senders"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["sender"].as_str().unwrap())
        .collect();
    assert_eq!(
        second_senders,
        vec!["same-b@example.org", "same-c@example.org"]
    );
    let second_cursor = second["next_cursor"]
        .as_str()
        .expect("second page cursor")
        .to_owned();
    assert_ne!(second_cursor, first_cursor);

    let third = request(
        state,
        Method::GET,
        &format!("/api/views/screener?limit=2&cursor={second_cursor}"),
        Some(&sid),
        false,
        None,
    )
    .await;
    assert_eq!(third.status(), StatusCode::OK);
    let third = json_body(third).await;
    let third_senders: Vec<&str> = third["senders"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["sender"].as_str().unwrap())
        .collect();
    assert_eq!(third_senders, vec!["oldest@example.org"]);
    assert!(third["next_cursor"].is_null());
}

#[tokio::test]
async fn screener_view_sorts_by_latest_pending_received_at_with_first_seen_fallback() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let now = Utc::now();
    let imported_old = chrono::DateTime::parse_from_rfc3339("2014-02-03T04:05:06Z")
        .unwrap()
        .with_timezone(&Utc);

    seed_rule(
        &state,
        user_id,
        "recent@example.org",
        "pending",
        None,
        now - Duration::days(10),
    )
    .await;
    set_latest_pending_received_at(&state, user_id, "recent@example.org", now).await;

    seed_rule(
        &state,
        user_id,
        "old-import@example.org",
        "pending",
        None,
        now,
    )
    .await;
    set_latest_pending_received_at(&state, user_id, "old-import@example.org", imported_old).await;

    seed_rule(
        &state,
        user_id,
        "fallback@example.org",
        "pending",
        None,
        now - Duration::hours(1),
    )
    .await;

    let tie_time = now - Duration::hours(2);
    for sender in ["tie-b@example.org", "tie-a@example.org"] {
        seed_rule(
            &state,
            user_id,
            sender,
            "pending",
            None,
            now - Duration::days(1),
        )
        .await;
        set_latest_pending_received_at(&state, user_id, sender, tie_time).await;
    }

    let resp = request(
        state,
        Method::GET,
        "/api/views/screener",
        Some(&sid),
        false,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    let got: Vec<&str> = json["senders"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["sender"].as_str().unwrap())
        .collect();
    assert_eq!(
        got,
        vec![
            "recent@example.org",
            "fallback@example.org",
            "tie-a@example.org",
            "tie-b@example.org",
            "old-import@example.org",
        ]
    );
}

#[tokio::test]
async fn screener_view_accepts_v1_cursor_and_emits_v2_cursor() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let now = Utc::now();
    for (sender, first_seen_at) in [
        ("new@example.org", now),
        ("middle@example.org", now - Duration::minutes(1)),
        ("old@example.org", now - Duration::minutes(2)),
    ] {
        seed_rule(&state, user_id, sender, "pending", None, first_seen_at).await;
    }

    let first = request(
        state.clone(),
        Method::GET,
        "/api/views/screener?limit=1",
        Some(&sid),
        false,
        None,
    )
    .await;
    assert_eq!(first.status(), StatusCode::OK);
    let first = json_body(first).await;
    let v2_cursor = first["next_cursor"].as_str().expect("v2 cursor");
    let decoded_v2 = String::from_utf8(URL_SAFE_NO_PAD.decode(v2_cursor).unwrap()).unwrap();
    assert!(decoded_v2.starts_with("2\n"), "cursor was {decoded_v2:?}");

    let v1_cursor = URL_SAFE_NO_PAD.encode(format!(
        "{}\n{}",
        (now - Duration::minutes(1)).to_rfc3339(),
        "middle@example.org"
    ));
    let after_v1 = request(
        state,
        Method::GET,
        &format!("/api/views/screener?limit=1&cursor={v1_cursor}"),
        Some(&sid),
        false,
        None,
    )
    .await;
    assert_eq!(after_v1.status(), StatusCode::OK);
    let after_v1 = json_body(after_v1).await;
    let senders = after_v1["senders"].as_array().unwrap();
    assert_eq!(senders.len(), 1);
    assert_eq!(senders[0]["sender"], "old@example.org");
}

#[tokio::test]
async fn screener_view_rejects_invalid_cursor() {
    let (state, key) = fixture_state().await;
    let (_, sid) = seed_session(&state, &key, "alice@example.org").await;
    let resp = request(
        state,
        Method::GET,
        "/api/views/screener?cursor=not-a-cursor",
        Some(&sid),
        false,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn screener_view_derives_previews_from_body_when_cached_provider_preview_is_empty() {
    const LONG_TEXT_BODY: &str = "   First line from text body.\n\nSecond line with extra spacing before a deliberately long suffix: 0123456789 0123456789 0123456789 0123456789 0123456789 0123456789 0123456789 0123456789 0123456789 0123456789 0123456789 0123456789 0123456789 0123456789 0123456789";

    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "screener-preview@example.org").await;
    let (state, _backend) = state_with_cache_mail(
        &state,
        user_id,
        vec![
            raw_screener_message(
                "email-text",
                "text@example.org",
                "Text body fallback",
                "",
                LONG_TEXT_BODY,
                "text/plain",
                1_700_000_010,
            ),
            raw_screener_message(
                "email-html",
                "html@example.org",
                "HTML body fallback",
                "",
                r#"<div><p>Order <strong>Ready</strong></p><table><tr><td>Total</td><td>$55.08</td></tr></table><img src="https://tracker.example/open.gif" width="1" height="1"><p>Thanks for reading.</p></div>"#,
                "text/html",
                1_700_000_020,
            ),
            raw_screener_message(
                "email-empty",
                "empty@example.org",
                "Empty body stays empty",
                "",
                " \n\t ",
                "text/plain",
                1_700_000_030,
            ),
        ],
    )
    .await;
    let now = Utc::now();
    for sender in ["text@example.org", "html@example.org", "empty@example.org"] {
        seed_rule(&state, user_id, sender, "pending", None, now).await;
    }

    let resp = request(
        state,
        Method::GET,
        "/api/views/screener",
        Some(&sid),
        false,
        None,
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    let senders = json["senders"].as_array().expect("screener senders");

    let text_sender = sender(senders, "text@example.org");
    let collapsed_text = LONG_TEXT_BODY
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let expected_text = collapsed_text.chars().take(200).collect::<String>();
    assert_eq!(text_sender["latest_preview"]["preview"], expected_text);
    assert_eq!(text_sender["emails"][0]["preview"], expected_text);
    assert!(
        !text_sender["latest_preview"]["preview"]
            .as_str()
            .unwrap()
            .starts_with(char::is_whitespace)
    );

    let html_sender = sender(senders, "html@example.org");
    let expected_html = "Order Ready Total$55.08Thanks for reading.";
    assert_eq!(html_sender["latest_preview"]["preview"], expected_html);
    assert_eq!(html_sender["emails"][0]["preview"], expected_html);

    let empty_sender = sender(senders, "empty@example.org");
    assert_eq!(empty_sender["latest_preview"]["preview"], "");
    assert_eq!(empty_sender["emails"][0]["preview"], "");
}

#[tokio::test]
async fn screener_view_fetches_messages_only_once_per_sender_in_cache_path() {
    let messages = (0..12)
        .map(|index| {
            raw_screener_message(
                &format!("bulk-email-{index:02}"),
                "bulk@example.org",
                &format!("Bulk message {index:02}"),
                &format!("Cached preview {index:02}"),
                "body already imported by cache",
                "text/plain",
                1_700_000_100 - i64::from(index),
            )
        })
        .collect::<Vec<_>>();

    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "screener-bulk@example.org").await;
    let (state, backend) = state_with_cache_mail(&state, user_id, messages).await;
    seed_rule(
        &state,
        user_id,
        "bulk@example.org",
        "pending",
        None,
        Utc::now(),
    )
    .await;

    let resp = request(
        state,
        Method::GET,
        "/api/views/screener",
        Some(&sid),
        false,
        None,
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    let senders = json["senders"].as_array().expect("screener senders");
    let bulk_sender = sender(senders, "bulk@example.org");
    assert_eq!(bulk_sender["message_count"], 12);
    assert_eq!(bulk_sender["emails"].as_array().unwrap().len(), 12);
    assert_eq!(bulk_sender["emails"][0]["email_id"], "bulk-email-00");
    assert_eq!(
        bulk_sender["latest_preview"]["preview"],
        "Cached preview 00"
    );
    assert_eq!(bulk_sender["emails"][0]["preview"], "Cached preview 00");
    assert_eq!(bulk_sender["emails"][1]["email_id"], "bulk-email-01");
    assert_eq!(bulk_sender["emails"][1]["preview"], "Cached preview 01");

    assert_eq!(
        backend.get_calls(),
        (0..12)
            .map(|index| format!("bulk-email-{index:02}"))
            .collect::<Vec<_>>(),
        "cache-backed screener enrichment should hydrate each listed backend id once"
    );
}

#[tokio::test]
async fn screener_view_prefers_existing_cached_provider_preview_over_body_fallback() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "screener-jmap@example.org").await;
    let (state, _backend) = state_with_cache_mail(
        &state,
        user_id,
        vec![raw_screener_message(
            "email-provider-preview",
            "jmap@example.org",
            "Existing preview",
            "Provider preview wins.",
            "Body fallback should not replace the provider preview.",
            "text/plain",
            1_700_000_010,
        )],
    )
    .await;
    seed_rule(
        &state,
        user_id,
        "jmap@example.org",
        "pending",
        None,
        Utc::now(),
    )
    .await;

    let resp = request(
        state,
        Method::GET,
        "/api/views/screener",
        Some(&sid),
        false,
        None,
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    let senders = json["senders"].as_array().expect("screener senders");
    let jmap_sender = sender(senders, "jmap@example.org");
    assert_eq!(
        jmap_sender["latest_preview"]["preview"],
        "Provider preview wins."
    );
    assert_eq!(
        jmap_sender["emails"][0]["preview"],
        "Provider preview wins."
    );
}

#[tokio::test]
async fn allowed_senders_view_returns_current_user_allowed_rows_with_timestamps() {
    let (state, key) = fixture_state().await;
    let (alice_id, alice_sid) = seed_session(&state, &key, "alice@example.org").await;
    let (bob_id, _) = seed_session(&state, &key, "bob@example.org").await;
    let now = Utc::now();
    seed_rule_with_decided_at(
        &state,
        alice_id,
        "old-allowed@example.org",
        "allow",
        Some("feed"),
        now - Duration::days(3),
        Some(now - Duration::hours(2)),
    )
    .await;
    seed_rule(
        &state,
        alice_id,
        "pending@example.org",
        "pending",
        None,
        now,
    )
    .await;
    seed_rule(&state, alice_id, "denied@example.org", "deny", None, now).await;
    seed_rule_with_decided_at(
        &state,
        alice_id,
        "NEW-ALLOWED@EXAMPLE.ORG",
        "allow",
        Some("papertrail"),
        now - Duration::days(1),
        Some(now),
    )
    .await;
    seed_rule(
        &state,
        alice_id,
        "legacy-allowed@example.org",
        "allow",
        Some("imbox"),
        now - Duration::hours(1),
    )
    .await;
    seed_rule(
        &state,
        bob_id,
        "bob-allowed@example.org",
        "allow",
        Some("imbox"),
        now,
    )
    .await;

    let resp = request(
        state,
        Method::GET,
        "/api/views/screener/allowed",
        Some(&alice_sid),
        false,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    let allowed = json["allowed"].as_array().unwrap();
    assert_eq!(allowed.len(), 3);
    assert_eq!(allowed[0]["sender_address"], "new-allowed@example.org");
    assert_eq!(allowed[0]["classify_as"], "papertrail");
    assert!(allowed[0]["first_seen_at"].is_string());
    assert!(allowed[0]["decided_at"].is_string());
    assert_eq!(allowed[1]["sender_address"], "legacy-allowed@example.org");
    assert_eq!(allowed[1]["classify_as"], "imbox");
    assert!(allowed[1]["decided_at"].is_null());
    assert_eq!(allowed[2]["sender_address"], "old-allowed@example.org");
    assert_eq!(allowed[2]["classify_as"], "feed");
}

#[tokio::test]
async fn allowed_senders_view_requires_auth() {
    let (state, _) = fixture_state().await;
    let resp = request(
        state,
        Method::GET,
        "/api/views/screener/allowed",
        None,
        false,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn denied_senders_view_returns_current_user_denied_rows() {
    let (state, key) = fixture_state().await;
    let (alice_id, alice_sid) = seed_session(&state, &key, "alice@example.org").await;
    let (bob_id, _) = seed_session(&state, &key, "bob@example.org").await;
    let now = Utc::now();
    seed_rule(
        &state,
        alice_id,
        "old-denied@example.org",
        "deny",
        None,
        now - Duration::hours(2),
    )
    .await;
    seed_rule(
        &state,
        alice_id,
        "allowed@example.org",
        "allow",
        Some("imbox"),
        now,
    )
    .await;
    seed_rule(
        &state,
        alice_id,
        "new-denied@example.org",
        "deny",
        None,
        now,
    )
    .await;
    seed_rule(&state, bob_id, "bob-denied@example.org", "deny", None, now).await;
    sqlx::query(
        "UPDATE screener_rules SET decided_at = first_seen_at WHERE user_id = ?1 AND decision = 'deny'",
    )
    .bind(alice_id)
    .execute(&state.db)
    .await
    .unwrap();

    let resp = request(
        state,
        Method::GET,
        "/api/views/screener/denied",
        Some(&alice_sid),
        false,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    let denied = json["denied"].as_array().unwrap();
    assert_eq!(denied.len(), 2);
    assert_eq!(denied[0]["sender_address"], "new-denied@example.org");
    assert!(denied[0]["denied_at"].is_string());
    assert_eq!(denied[1]["sender_address"], "old-denied@example.org");
}

#[tokio::test]
async fn undo_deny_approves_current_user_denied_rule_with_default_classification() {
    let (state, key) = fixture_state().await;
    let (alice_id, alice_sid) = seed_session(&state, &key, "alice@example.org").await;
    let (bob_id, _) = seed_session(&state, &key, "bob@example.org").await;
    let now = Utc::now();
    seed_rule(&state, alice_id, "spam@example.org", "deny", None, now).await;
    seed_rule(&state, bob_id, "spam@example.org", "deny", None, now).await;
    seed_rule(
        &state,
        alice_id,
        "allowed@example.org",
        "allow",
        Some("feed"),
        now,
    )
    .await;

    let resp = request(
        state.clone(),
        Method::POST,
        "/api/screener/spam%40example.org/undo-deny",
        Some(&alice_sid),
        true,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert_eq!(json["status"], "approved");
    assert_eq!(json["classify_as"], "imbox");

    let alice_spam: (String, Option<String>) = sqlx::query_as(
        "SELECT decision, classify_as FROM screener_rules WHERE user_id = ?1 AND sender_address = 'spam@example.org'",
    )
    .bind(alice_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(alice_spam.0, "allow");
    assert_eq!(alice_spam.1.as_deref(), Some("imbox"));

    let bob_spam: (String, Option<String>) = sqlx::query_as(
        "SELECT decision, classify_as FROM screener_rules WHERE user_id = ?1 AND sender_address = 'spam@example.org'",
    )
    .bind(bob_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(bob_spam.0, "deny");
    assert!(bob_spam.1.is_none());

    let allowed_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM screener_rules WHERE user_id = ?1 AND sender_address = 'allowed@example.org'",
    )
    .bind(alice_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(allowed_count, 1);
}

#[tokio::test]
async fn undo_deny_accepts_classify_as_and_backfills_history() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let now = Utc::now();
    seed_rule(&state, user_id, "spam@example.org", "deny", None, now).await;
    let backfill = Arc::new(FakeBackfill::default());

    let resp = request_with_backfill(
        state.clone(),
        backfill.clone(),
        Method::POST,
        "/api/screener/spam%40example.org/undo-deny",
        Some(&sid),
        true,
        Some(r#"{"classify_as":"papertrail"}"#),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert_eq!(json["status"], "approved");
    assert_eq!(json["classify_as"], "papertrail");

    let row: (String, Option<String>) = sqlx::query_as(
        "SELECT decision, classify_as FROM screener_rules WHERE user_id = ?1 AND sender_address = 'spam@example.org'",
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(row.0, "allow");
    assert_eq!(row.1.as_deref(), Some("papertrail"));

    let calls = backfill.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0],
        BackfillCall {
            user_id,
            sender: "spam@example.org".to_string(),
            decision: ScreenerDecision::Approve,
            classify_as: Some(Classification::Papertrail),
        }
    );
}

#[tokio::test]
async fn undo_deny_rejects_invalid_classify_as() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let now = Utc::now();
    seed_rule(&state, user_id, "spam@example.org", "deny", None, now).await;
    let backfill = Arc::new(FakeBackfill::default());

    let resp = request_with_backfill(
        state.clone(),
        backfill.clone(),
        Method::POST,
        "/api/screener/spam%40example.org/undo-deny",
        Some(&sid),
        true,
        Some(r#"{"classify_as":"other"}"#),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let json = json_body(resp).await;
    assert_eq!(json["error"], "invalid_classify_as");
    assert!(backfill.calls.lock().unwrap().is_empty());

    let row: (String, Option<String>) = sqlx::query_as(
        "SELECT decision, classify_as FROM screener_rules WHERE user_id = ?1 AND sender_address = 'spam@example.org'",
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(row.0, "deny");
    assert!(row.1.is_none());
}

#[tokio::test]
async fn undo_deny_missing_csrf_returns_403() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let resp = request(
        state,
        Method::POST,
        "/api/screener/spam%40example.org/undo-deny",
        Some(&sid),
        false,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn approve_creates_or_updates_row_and_classify_as() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    seed_rule(
        &state,
        user_id,
        "news@example.org",
        "pending",
        None,
        Utc::now(),
    )
    .await;
    let resp = request(state.clone(), Method::POST, "/api/screener/decisions", Some(&sid), true, Some(r#"{"sender":" News@Example.ORG ","decision":"approve","classify_as":"feed","apply_to_history":false}"#)).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert_eq!(json["sender"], "news@example.org");
    assert_eq!(json["decision"], "approve");
    assert_eq!(json["classify_as"], "feed");
    let row: (String, Option<String>, Option<String>) = sqlx::query_as("SELECT decision, classify_as, decided_at FROM screener_rules WHERE user_id = ?1 AND sender_address = 'news@example.org'").bind(user_id).fetch_one(&state.db).await.unwrap();
    assert_eq!(row.0, "allow");
    assert_eq!(row.1.as_deref(), Some("feed"));
    assert!(row.2.is_some());

    let audit: (i64, String, String) =
        sqlx::query_as("SELECT user_id, action, payload_json FROM audit_log")
            .fetch_one(&state.db)
            .await
            .unwrap();
    assert_eq!(audit.0, user_id);
    assert_eq!(audit.1, "screener.decision");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&audit.2).unwrap(),
        serde_json::json!({
            "sender": "news@example.org",
            "decision": "approve",
            "classify_as": "feed",
            "apply_to_history": false,
        })
    );
}

#[tokio::test]
async fn approve_clears_pin_for_sender_without_other_pin_sources() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let account_id = seed_mail_account(&state, user_id, "alice@example.org").await;
    seed_rule(
        &state,
        user_id,
        "news@example.org",
        "pending",
        None,
        Utc::now(),
    )
    .await;
    seed_cached_message(&state, account_id, "news-1", "news@example.org", true).await;

    let resp = request(state.clone(), Method::POST, "/api/screener/decisions", Some(&sid), true, Some(r#"{"sender":"news@example.org","decision":"approve","classify_as":"feed","apply_to_history":false}"#)).await;

    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(cached_pin(&state, "news-1").await, 0);
}

#[tokio::test]
async fn deny_clears_pin_for_sender_without_other_pin_sources() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let account_id = seed_mail_account(&state, user_id, "alice@example.org").await;
    seed_rule(
        &state,
        user_id,
        "spam@example.org",
        "pending",
        None,
        Utc::now(),
    )
    .await;
    seed_cached_message(&state, account_id, "spam-1", "spam@example.org", true).await;

    let resp = request(
        state.clone(),
        Method::POST,
        "/api/screener/decisions",
        Some(&sid),
        true,
        Some(r#"{"sender":"spam@example.org","decision":"deny","apply_to_history":false}"#),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(cached_pin(&state, "spam-1").await, 0);
}

#[tokio::test]
async fn approve_without_classify_as_defaults_to_imbox() {
    let (state, key) = fixture_state().await;
    let (_, sid) = seed_session(&state, &key, "alice@example.org").await;
    let resp = request(
        state,
        Method::POST,
        "/api/screener/decisions",
        Some(&sid),
        true,
        Some(r#"{"sender":"person@example.org","decision":"approve","apply_to_history":false}"#),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert_eq!(json["classify_as"], "imbox");
}

#[tokio::test]
async fn deny_creates_or_updates_row() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    seed_rule(
        &state,
        user_id,
        "spam@example.org",
        "allow",
        Some("imbox"),
        Utc::now(),
    )
    .await;
    let resp = request(
        state.clone(),
        Method::POST,
        "/api/screener/decisions",
        Some(&sid),
        true,
        Some(r#"{"sender":"spam@example.org","decision":"deny","apply_to_history":false}"#),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert_eq!(json["sender"], "spam@example.org");
    assert_eq!(json["decision"], "deny");
    assert!(json["classify_as"].is_null());
    let row: (String, Option<String>, Option<String>) = sqlx::query_as("SELECT decision, classify_as, decided_at FROM screener_rules WHERE user_id = ?1 AND sender_address = 'spam@example.org'").bind(user_id).fetch_one(&state.db).await.unwrap();
    assert_eq!(row.0, "deny");
    assert!(row.1.is_none());
    assert!(row.2.is_some());
}

#[tokio::test]
async fn deny_rejects_classify_as() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    seed_rule(
        &state,
        user_id,
        "spam@example.org",
        "allow",
        Some("feed"),
        Utc::now(),
    )
    .await;

    let resp = request(
        state.clone(),
        Method::POST,
        "/api/screener/decisions",
        Some(&sid),
        true,
        Some(r#"{"sender":"spam@example.org","decision":"deny","classify_as":"imbox","apply_to_history":false}"#),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let json = json_body(resp).await;
    assert_eq!(json["error"], "invalid_classify_as");

    let row: (String, Option<String>) = sqlx::query_as(
        "SELECT decision, classify_as FROM screener_rules WHERE user_id = ?1 AND sender_address = 'spam@example.org'",
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(row.0, "allow");
    assert_eq!(row.1.as_deref(), Some("feed"));
}

#[tokio::test]
async fn decision_missing_csrf_returns_403() {
    let (state, key) = fixture_state().await;
    let (_, sid) = seed_session(&state, &key, "alice@example.org").await;
    let resp = request(
        state,
        Method::POST,
        "/api/screener/decisions",
        Some(&sid),
        false,
        Some(r#"{"sender":"x@example.org","decision":"deny","apply_to_history":false}"#),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn decision_without_auth_returns_401() {
    let (state, _) = fixture_state().await;
    let resp = request(
        state,
        Method::POST,
        "/api/screener/decisions",
        None,
        true,
        Some(r#"{"sender":"x@example.org","decision":"deny","apply_to_history":false}"#),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn invalid_decision_or_classify_as_returns_400() {
    let (state, key) = fixture_state().await;
    let (_, sid) = seed_session(&state, &key, "alice@example.org").await;
    let bad_decision = request(
        state.clone(),
        Method::POST,
        "/api/screener/decisions",
        Some(&sid),
        true,
        Some(r#"{"sender":"x@example.org","decision":"maybe","apply_to_history":false}"#),
    )
    .await;
    assert_eq!(bad_decision.status(), StatusCode::BAD_REQUEST);
    let bad_classify = request(state, Method::POST, "/api/screener/decisions", Some(&sid), true, Some(r#"{"sender":"x@example.org","decision":"approve","classify_as":"other","apply_to_history":false}"#)).await;
    assert_eq!(bad_classify.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn apply_to_history_calls_fake_backfill_once() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let backfill = Arc::new(FakeBackfill::default());
    let resp = request_with_backfill(state, backfill.clone(), Method::POST, "/api/screener/decisions", Some(&sid), true, Some(r#"{"sender":" Sender@Example.ORG ","decision":"approve","classify_as":"papertrail","apply_to_history":true}"#)).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let calls = backfill.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0],
        BackfillCall {
            user_id,
            sender: "sender@example.org".to_string(),
            decision: ScreenerDecision::Approve,
            classify_as: Some(Classification::Papertrail)
        }
    );
}

#[tokio::test]
async fn decision_response_undo_payload_snapshots_previous_rule() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "undo-existing@example.org").await;
    let first_seen_at = Utc::now() - Duration::days(3);
    seed_rule(
        &state,
        user_id,
        "sender@example.org",
        "allow",
        Some("feed"),
        first_seen_at,
    )
    .await;

    let resp = request(
        state.clone(),
        Method::POST,
        "/api/screener/decisions",
        Some(&sid),
        true,
        Some(r#"{"sender":"sender@example.org","decision":"deny","apply_to_history":false}"#),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert_eq!(json["undo"]["action"], "screener.decision");
    let undo_id = json["undo"]["id"].as_str().unwrap();

    let payload_json: String =
        sqlx::query_scalar("SELECT payload_json FROM undo_actions WHERE id = ?1")
            .bind(undo_id)
            .fetch_one(&state.db)
            .await
            .unwrap();
    let payload: serde_json::Value = serde_json::from_str(&payload_json).unwrap();
    assert_eq!(payload["sender"], "sender@example.org");
    assert_eq!(payload["previous_rule"]["decision"], "allow");
    assert_eq!(payload["previous_rule"]["classify_as"], "feed");
    assert!(payload["previous_rule"]["decided_at"].is_null());
    assert_eq!(
        payload["previous_rule"]["first_seen_at"],
        serde_json::json!(first_seen_at)
    );
}

#[tokio::test]
async fn decision_response_undo_payload_marks_new_sender_for_delete() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "undo-new@example.org").await;

    let resp = request(
        state.clone(),
        Method::POST,
        "/api/screener/decisions",
        Some(&sid),
        true,
        Some(r#"{"sender":"new@example.org","decision":"approve","apply_to_history":false}"#),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert_eq!(json["undo"]["action"], "screener.decision");
    let undo_id = json["undo"]["id"].as_str().unwrap();

    let payload_json: String =
        sqlx::query_scalar("SELECT payload_json FROM undo_actions WHERE id = ?1")
            .bind(undo_id)
            .fetch_one(&state.db)
            .await
            .unwrap();
    let payload: serde_json::Value = serde_json::from_str(&payload_json).unwrap();
    assert_eq!(payload["sender"], "new@example.org");
    assert!(payload["previous_rule"].is_null());
}

#[tokio::test]
async fn backfill_failure_returns_500_without_persisting_decision_audit_or_undo() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "backfill-fail@example.org").await;
    let backfill = Arc::new(FakeBackfill::failing());

    let resp = request_with_backfill(
        state.clone(),
        backfill.clone(),
        Method::POST,
        "/api/screener/decisions",
        Some(&sid),
        true,
        Some(r#"{"sender":"sender@example.org","decision":"approve","classify_as":"papertrail","apply_to_history":true}"#),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let row_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM screener_rules WHERE user_id = ?1 AND sender_address = 'sender@example.org'",
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(row_count, 0);
    assert_eq!(backfill.calls.lock().unwrap().len(), 1);

    let audit_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_log")
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(audit_count, 0);
    let undo_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM undo_actions")
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(undo_count, 0);
}

#[tokio::test]
async fn backfill_failure_preserves_previous_screener_rule() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "backfill-existing@example.org").await;
    let first_seen_at = Utc::now() - Duration::days(2);
    seed_rule(
        &state,
        user_id,
        "sender@example.org",
        "pending",
        None,
        first_seen_at,
    )
    .await;
    let backfill = Arc::new(FakeBackfill::failing());

    let resp = request_with_backfill(
        state.clone(),
        backfill.clone(),
        Method::POST,
        "/api/screener/decisions",
        Some(&sid),
        true,
        Some(r#"{"sender":"sender@example.org","decision":"approve","classify_as":"papertrail","apply_to_history":true}"#),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let row: (
        String,
        Option<String>,
        Option<chrono::DateTime<Utc>>,
        chrono::DateTime<Utc>,
    ) = sqlx::query_as(
        "SELECT decision, classify_as, decided_at, first_seen_at FROM screener_rules \
             WHERE user_id = ?1 AND sender_address = 'sender@example.org'",
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(row.0, "pending");
    assert_eq!(row.1, None);
    assert_eq!(row.2, None);
    assert_eq!(row.3, first_seen_at);
    assert_eq!(backfill.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn backfill_failure_rolls_back_decision_audit_and_undo_together() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "backfill-atomic@example.org").await;
    let first_seen_at = Utc::now() - Duration::days(2);
    seed_rule(
        &state,
        user_id,
        "sender@example.org",
        "pending",
        None,
        first_seen_at,
    )
    .await;
    let backfill = Arc::new(FakeBackfill::failing());

    let resp = request_with_backfill(
        state.clone(),
        backfill.clone(),
        Method::POST,
        "/api/screener/decisions",
        Some(&sid),
        true,
        Some(r#"{"sender":"sender@example.org","decision":"deny","apply_to_history":true}"#),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(backfill.calls.lock().unwrap().len(), 1);

    let row: (
        String,
        Option<String>,
        Option<chrono::DateTime<Utc>>,
        chrono::DateTime<Utc>,
    ) = sqlx::query_as(
        "SELECT decision, classify_as, decided_at, first_seen_at FROM screener_rules \
             WHERE user_id = ?1 AND sender_address = 'sender@example.org'",
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(row.0, "pending");
    assert_eq!(row.1, None);
    assert_eq!(row.2, None);
    assert_eq!(row.3, first_seen_at);

    let audit_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_log")
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(audit_count, 0);
    let undo_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM undo_actions")
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(undo_count, 0);
}

#[tokio::test]
async fn invalid_sender_returns_400_without_persisting_or_backfill() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "invalid-sender@example.org").await;

    for body in [
        r#"{"sender":"","decision":"deny","apply_to_history":true}"#,
        r#"{"sender":"not-an-email","decision":"deny","apply_to_history":true}"#,
        r#"{"sender":"bad domain@example.org","decision":"deny","apply_to_history":true}"#,
        r#"{"sender":"bad@example","decision":"deny","apply_to_history":true}"#,
    ] {
        let backfill = Arc::new(FakeBackfill::default());
        let resp = request_with_backfill(
            state.clone(),
            backfill.clone(),
            Method::POST,
            "/api/screener/decisions",
            Some(&sid),
            true,
            Some(body),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "{body}");
        let json = json_body(resp).await;
        assert_eq!(json["error"], "invalid_sender", "{body}");
        assert!(backfill.calls.lock().unwrap().is_empty(), "{body}");
    }

    let rule_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM screener_rules")
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(rule_count, 0);
}

#[tokio::test]
async fn malformed_decision_bodies_return_400_without_persisting_or_backfill() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "bad-body@example.org").await;

    for body in [
        "not-json",
        r#"{"sender":"sender@example.org","decision":"approve"}"#,
        r#"{"sender":"sender@example.org","decision":"approve","apply_to_history":"yes"}"#,
        r#"{"decision":"approve","apply_to_history":false}"#,
    ] {
        let backfill = Arc::new(FakeBackfill::default());
        let resp = request_with_backfill(
            state.clone(),
            backfill.clone(),
            Method::POST,
            "/api/screener/decisions",
            Some(&sid),
            true,
            Some(body),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "{body}");
        let json = json_body(resp).await;
        assert_eq!(json["error"], "invalid_decision_body", "{body}");
        assert!(backfill.calls.lock().unwrap().is_empty(), "{body}");
    }

    let rule_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM screener_rules")
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(rule_count, 0);
}
