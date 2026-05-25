#![allow(dead_code)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use hail_jmap::jmap_client::Error as JmapClientError;
use hail_jmap::jmap_client::core::set::SetErrorType;
use secrecy::{ExposeSecret, SecretString};
use sqlx::SqlitePool;
use tokio::sync::Barrier;

#[path = "../src/app_events.rs"]
mod app_events;

#[path = "../src/crypto.rs"]
#[allow(dead_code)]
mod crypto;

#[path = "../src/jmap_helpers.rs"]
mod jmap_helpers;

#[path = "../src/scheduler.rs"]
mod scheduler;

use scheduler::live::{
    LiveSendSubmitter, classify_jmap_set_error_type_for_test, classify_jmap_submit_error_for_test,
};
use scheduler::{SendSubmitError, SendSubmitter, process_due_scheduled_sends};

#[derive(Debug, Default)]
struct EchoTokenDecryptor;

impl crypto::TokenDecryptor for EchoTokenDecryptor {
    fn decrypt(&self, enc: &[u8]) -> Result<SecretString, crypto::DecryptError> {
        if enc == b"decrypt-fail" {
            return Err(crypto::DecryptError::AuthFailed);
        }
        String::from_utf8(enc.to_vec())
            .map(SecretString::from)
            .map_err(|_| crypto::DecryptError::Malformed("plaintext is not valid utf-8"))
    }
}

#[derive(Debug, Default)]
struct FakeSendSubmitter {
    calls: Mutex<Vec<(i64, String)>>,
    results_by_draft: Mutex<HashMap<String, Result<Option<String>, SendSubmitError>>>,
}

impl FakeSendSubmitter {
    fn set_result(&self, draft_email_id: &str, result: Result<Option<String>, SendSubmitError>) {
        self.results_by_draft
            .lock()
            .expect("results mutex")
            .insert(draft_email_id.to_string(), result);
    }

    fn calls(&self) -> Vec<(i64, String)> {
        self.calls.lock().expect("calls mutex").clone()
    }
}

#[async_trait]
impl SendSubmitter for FakeSendSubmitter {
    async fn submit_draft(
        &self,
        _scheduled_send_id: i64,
        user_id: i64,
        draft_email_id: &str,
    ) -> Result<Option<String>, SendSubmitError> {
        self.calls
            .lock()
            .expect("calls mutex")
            .push((user_id, draft_email_id.to_string()));
        self.results_by_draft
            .lock()
            .expect("results mutex")
            .remove(draft_email_id)
            .unwrap_or_else(|| Ok(Some(format!("submission-{draft_email_id}"))))
    }
}

#[derive(Debug)]
struct BlockingSubmitter {
    calls: Mutex<Vec<(i64, String)>>,
    first_call_entered: Barrier,
    release_first_call: Barrier,
}

impl BlockingSubmitter {
    fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            first_call_entered: Barrier::new(2),
            release_first_call: Barrier::new(2),
        }
    }

    fn calls(&self) -> Vec<(i64, String)> {
        self.calls.lock().expect("calls mutex").clone()
    }
}

#[async_trait]
impl SendSubmitter for BlockingSubmitter {
    async fn submit_draft(
        &self,
        _scheduled_send_id: i64,
        user_id: i64,
        draft_email_id: &str,
    ) -> Result<Option<String>, SendSubmitError> {
        let is_first_call = {
            let mut calls = self.calls.lock().expect("calls mutex");
            calls.push((user_id, draft_email_id.to_string()));
            calls.len() == 1
        };
        if is_first_call {
            self.first_call_entered.wait().await;
            self.release_first_call.wait().await;
        }
        Ok(Some(format!("submission-{draft_email_id}")))
    }
}

static DB_COUNTER: AtomicU64 = AtomicU64::new(0);

fn fresh_db_url() -> (String, PathBuf) {
    let mut path = std::env::temp_dir();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    let pid = std::process::id();
    let counter = DB_COUNTER.fetch_add(1, Ordering::SeqCst);
    path.push(format!(
        "hail-worker-send-later-scheduler-test-{pid}-{nanos}-{counter}.sqlite"
    ));
    let url = format!("sqlite://{}", path.display());
    (url, path)
}

struct TempDb(PathBuf);

impl Drop for TempDb {
    fn drop(&mut self) {
        let _ = &self.0;
    }
}

async fn setup_db() -> (SqlitePool, TempDb, i64) {
    let (url, path) = fresh_db_url();
    let guard = TempDb(path);
    let pool = hail_db::connect(&url).await.expect("connect");
    hail_db::migrate(&pool).await.expect("migrate");

    sqlx::query("INSERT INTO users (email, jmap_account_id, created_at) VALUES (?, ?, ?)")
        .bind("alice@example.com")
        .bind("acct-alice")
        .bind("2026-01-01T00:00:00Z")
        .execute(&pool)
        .await
        .expect("insert user");
    let user_id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE email = ?")
        .bind("alice@example.com")
        .fetch_one(&pool)
        .await
        .expect("fetch user id");
    (pool, guard, user_id)
}

async fn insert_scheduled_send_with_status(
    pool: &SqlitePool,
    user_id: i64,
    draft_email_id: &str,
    send_at: DateTime<Utc>,
    status: &str,
) -> i64 {
    sqlx::query(
        "INSERT INTO scheduled_sends (user_id, draft_email_id, send_at, status, created_at) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(user_id)
    .bind(draft_email_id)
    .bind(send_at)
    .bind(status)
    .bind("2026-01-01T00:00:00Z")
    .execute(pool)
    .await
    .expect("insert scheduled_send")
    .last_insert_rowid()
}

async fn set_scheduled_send_claimed_at(pool: &SqlitePool, id: i64, claimed_at: DateTime<Utc>) {
    sqlx::query("UPDATE scheduled_sends SET claimed_at = ? WHERE id = ?")
        .bind(claimed_at)
        .bind(id)
        .execute(pool)
        .await
        .expect("set scheduled_send claimed_at");
}

async fn set_scheduled_send_claimed_at_null(pool: &SqlitePool, id: i64) {
    sqlx::query("UPDATE scheduled_sends SET claimed_at = NULL WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .expect("clear scheduled_send claimed_at");
}

async fn insert_scheduled_send(
    pool: &SqlitePool,
    user_id: i64,
    draft_email_id: &str,
    send_at: DateTime<Utc>,
) -> i64 {
    insert_scheduled_send_with_status(pool, user_id, draft_email_id, send_at, "pending").await
}

async fn insert_session(
    pool: &SqlitePool,
    user_id: i64,
    id: &str,
    token: &[u8],
    expires_at: DateTime<Utc>,
    last_used_at: DateTime<Utc>,
) {
    sqlx::query(
        "INSERT INTO sessions \
         (id, user_id, jmap_token_enc, user_agent, expires_at, created_at, last_used_at) \
         VALUES (?, ?, ?, 'test', ?, ?, ?)",
    )
    .bind(id)
    .bind(user_id)
    .bind(token)
    .bind(expires_at)
    .bind("2026-01-01T00:00:00Z")
    .bind(last_used_at)
    .execute(pool)
    .await
    .expect("insert session");
}

async fn attach_scheduled_send_session(
    pool: &SqlitePool,
    scheduled_send_id: i64,
    session_id: &str,
    expires_at: DateTime<Utc>,
) {
    sqlx::query(
        "UPDATE scheduled_sends \
         SET auth_session_id = ?, auth_session_expires_at = ? \
         WHERE id = ?",
    )
    .bind(session_id)
    .bind(expires_at)
    .bind(scheduled_send_id)
    .execute(pool)
    .await
    .expect("attach scheduled send session");
}

async fn insert_scheduled_send_session(
    pool: &SqlitePool,
    user_id: i64,
    scheduled_send_id: i64,
    id: &str,
    token: &[u8],
    expires_at: DateTime<Utc>,
) {
    insert_session(pool, user_id, id, token, expires_at, Utc::now()).await;
    attach_scheduled_send_session(pool, scheduled_send_id, id, expires_at).await;
}

async fn scheduled_send_state(
    pool: &SqlitePool,
    id: i64,
) -> (String, Option<String>, Option<String>, Option<String>) {
    sqlx::query_as("SELECT status, claimed_at, sent_at, error FROM scheduled_sends WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("select scheduled_send state")
}

#[tokio::test]
async fn live_submitter_uses_newest_non_expired_session() {
    let (pool, _guard, user_id) = setup_db().await;
    let now = Utc::now();
    insert_session(
        &pool,
        user_id,
        "expired-newest",
        b"expired-token",
        now - Duration::minutes(1),
        now + Duration::hours(1),
    )
    .await;
    insert_session(
        &pool,
        user_id,
        "active-older",
        b"older-token",
        now + Duration::hours(1),
        now - Duration::minutes(10),
    )
    .await;
    insert_session(
        &pool,
        user_id,
        "active-newest",
        b"newest-token",
        now + Duration::hours(1),
        now - Duration::minutes(1),
    )
    .await;
    let submitter = LiveSendSubmitter::new(
        pool,
        "http://127.0.0.1:9/jmap".to_string(),
        Arc::new(EchoTokenDecryptor),
    );

    let (token, email) = submitter
        .latest_active_token_and_email(user_id)
        .await
        .expect("active token");

    assert_eq!(token.expose_secret(), "newest-token");
    assert_eq!(email, "alice@example.com");
}

#[tokio::test]
async fn live_submitter_missing_active_session_is_transient() {
    let (pool, _guard, user_id) = setup_db().await;
    let now = Utc::now();
    insert_session(
        &pool,
        user_id,
        "expired-only",
        b"expired-token",
        now - Duration::minutes(1),
        now,
    )
    .await;
    let submitter = LiveSendSubmitter::new(
        pool,
        "http://127.0.0.1:9/jmap".to_string(),
        Arc::new(EchoTokenDecryptor),
    );

    let err = submitter
        .latest_active_token_and_email(user_id)
        .await
        .expect_err("no active session");

    assert!(
        matches!(err, SendSubmitError::Transient(ref message) if message.contains("no active JMAP session")),
        "unexpected error: {err:?}"
    );
}

#[tokio::test]
async fn live_submitter_decrypt_failure_is_transient() {
    let (pool, _guard, user_id) = setup_db().await;
    let now = Utc::now();
    insert_session(
        &pool,
        user_id,
        "bad-token",
        b"decrypt-fail",
        now + Duration::hours(1),
        now,
    )
    .await;
    let submitter = LiveSendSubmitter::new(
        pool,
        "http://127.0.0.1:9/jmap".to_string(),
        Arc::new(EchoTokenDecryptor),
    );

    let err = submitter
        .latest_active_token_and_email(user_id)
        .await
        .expect_err("decrypt failure");

    assert!(
        matches!(err, SendSubmitError::Transient(ref message) if message.contains("decrypt JMAP token")),
        "unexpected error: {err:?}"
    );
}

#[tokio::test]
async fn scheduled_send_missing_auth_session_marks_auth_required() {
    let (pool, _guard, user_id) = setup_db().await;
    let now = Utc::now();
    let id =
        insert_scheduled_send(&pool, user_id, "draft-no-auth", now - Duration::minutes(1)).await;
    let submitter = LiveSendSubmitter::new(
        pool.clone(),
        "http://127.0.0.1:9/jmap".to_string(),
        Arc::new(EchoTokenDecryptor),
    );

    let sent = process_due_scheduled_sends(&pool, &submitter, now)
        .await
        .expect("process due");

    assert_eq!(sent, 0);
    let state = scheduled_send_state(&pool, id).await;
    assert_eq!(state.0, "auth_required");
    assert!(
        state
            .3
            .as_deref()
            .is_some_and(|message| message.contains("auth_required")),
        "unexpected state: {state:?}"
    );
}

#[tokio::test]
async fn scheduled_send_expired_auth_session_marks_auth_required() {
    let (pool, _guard, user_id) = setup_db().await;
    let now = Utc::now();
    let id = insert_scheduled_send(
        &pool,
        user_id,
        "draft-expired-auth",
        now - Duration::minutes(1),
    )
    .await;
    insert_scheduled_send_session(
        &pool,
        user_id,
        id,
        "expired-send-session",
        b"expired-token",
        now - Duration::minutes(1),
    )
    .await;
    let submitter = LiveSendSubmitter::new(
        pool.clone(),
        "http://127.0.0.1:9/jmap".to_string(),
        Arc::new(EchoTokenDecryptor),
    );

    let sent = process_due_scheduled_sends(&pool, &submitter, now)
        .await
        .expect("process due");

    assert_eq!(sent, 0);
    let state = scheduled_send_state(&pool, id).await;
    assert_eq!(state.0, "auth_required");
    assert!(
        state
            .3
            .as_deref()
            .is_some_and(|message| message.contains("fresh login")),
        "unexpected state: {state:?}"
    );
}

#[tokio::test]
async fn scheduled_send_uses_recorded_session_not_newest_browser_session() {
    let (pool, _guard, user_id) = setup_db().await;
    let now = Utc::now();
    let id = insert_scheduled_send(
        &pool,
        user_id,
        "draft-recorded-auth",
        now - Duration::minutes(1),
    )
    .await;
    insert_scheduled_send_session(
        &pool,
        user_id,
        id,
        "recorded-send-session",
        b"recorded-token",
        now + Duration::hours(1),
    )
    .await;
    insert_session(
        &pool,
        user_id,
        "active-newer-browser-session",
        b"newer-browser-token",
        now + Duration::hours(1),
        now + Duration::minutes(1),
    )
    .await;
    sqlx::query("UPDATE scheduled_sends SET status = 'processing' WHERE id = ?")
        .bind(id)
        .execute(&pool)
        .await
        .expect("mark processing");
    let submitter = LiveSendSubmitter::new(
        pool.clone(),
        "http://127.0.0.1:9/jmap".to_string(),
        Arc::new(EchoTokenDecryptor),
    );

    let (token, email) = submitter
        .scheduled_token_and_email(id, user_id, "draft-recorded-auth")
        .await
        .expect("scheduled token");

    assert_eq!(token.expose_secret(), "recorded-token");
    assert_eq!(email, "alice@example.com");
}

#[test]
fn live_submitter_classifies_jmap_submit_errors() {
    for error_type in [SetErrorType::RateLimit, SetErrorType::OverQuota] {
        let err = classify_jmap_set_error_type_for_test(&error_type);
        assert!(
            matches!(err, SendSubmitError::Transient(ref message) if message.contains(&error_type.to_string())),
            "{error_type:?} should be transient, got {err:?}"
        );
    }

    for error_type in [
        SetErrorType::Forbidden,
        SetErrorType::NotFound,
        SetErrorType::InvalidProperties,
        SetErrorType::ForbiddenFrom,
        SetErrorType::InvalidEmail,
        SetErrorType::TooManyRecipients,
        SetErrorType::NoRecipients,
        SetErrorType::InvalidRecipients,
        SetErrorType::ForbiddenMailFrom,
        SetErrorType::ForbiddenToSend,
    ] {
        let err = classify_jmap_set_error_type_for_test(&error_type);
        assert!(
            matches!(err, SendSubmitError::Permanent(ref message) if message.contains(&error_type.to_string())),
            "{error_type:?} should be permanent, got {err:?}"
        );
    }

    let server_err = classify_jmap_submit_error_for_test(JmapClientError::Server(
        "temporarily unavailable".to_string(),
    ));
    assert!(
        matches!(server_err, SendSubmitError::Transient(ref message) if message.contains("temporarily unavailable")),
        "server errors should be transient, got {server_err:?}"
    );

    let internal_err = classify_jmap_submit_error_for_test(JmapClientError::Internal(
        "malformed response".to_string(),
    ));
    assert!(
        matches!(internal_err, SendSubmitError::Permanent(ref message) if message.contains("malformed response")),
        "internal errors should be permanent, got {internal_err:?}"
    );
}

#[tokio::test]
async fn due_send_submits_and_marks_sent() {
    let (pool, _guard, user_id) = setup_db().await;
    let now = Utc::now();
    let id = insert_scheduled_send(&pool, user_id, "draft-due", now - Duration::minutes(1)).await;
    let submitter = FakeSendSubmitter::default();

    let sent = process_due_scheduled_sends(&pool, &submitter, now)
        .await
        .expect("process due");

    assert_eq!(sent, 1);
    assert_eq!(submitter.calls(), vec![(user_id, "draft-due".to_string())]);
    assert_eq!(
        scheduled_send_state(&pool, id).await,
        (
            "sent".to_string(),
            Some(now.to_rfc3339()),
            Some(now.to_rfc3339()),
            None
        )
    );
    let event: (String, String) = sqlx::query_as(
        "SELECT event_type, payload_json FROM app_events WHERE user_id = ? ORDER BY id DESC LIMIT 1",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .expect("app event");
    assert_eq!(event.0, "send.completed");
    let event_payload: serde_json::Value = serde_json::from_str(&event.1).expect("event payload");
    assert_eq!(event_payload["scheduled_send_id"], id);
    assert_eq!(event_payload["draft_email_id"], "draft-due");

    let audit: (String, String) = sqlx::query_as(
        "SELECT action, payload_json FROM audit_log WHERE user_id = ? ORDER BY id DESC LIMIT 1",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .expect("audit row");
    assert_eq!(audit.0, "compose.send_later.sent");
    let audit_payload: serde_json::Value = serde_json::from_str(&audit.1).expect("audit payload");
    assert_eq!(audit_payload["scheduled_send_id"], id);
    assert_eq!(audit_payload["draft_email_id"], "draft-due");
    assert_eq!(audit_payload["submission_id"], "submission-draft-due");
    assert_eq!(audit_payload["sent_at"], now.to_rfc3339());
}

#[tokio::test]
async fn due_at_exactly_now_submits() {
    let (pool, _guard, user_id) = setup_db().await;
    let now = Utc::now();
    let id = insert_scheduled_send(&pool, user_id, "draft-now", now).await;
    let submitter = FakeSendSubmitter::default();

    let sent = process_due_scheduled_sends(&pool, &submitter, now)
        .await
        .expect("process due");

    assert_eq!(sent, 1);
    assert_eq!(submitter.calls(), vec![(user_id, "draft-now".to_string())]);
    assert_eq!(scheduled_send_state(&pool, id).await.0, "sent");
}

#[tokio::test]
async fn future_send_not_touched() {
    let (pool, _guard, user_id) = setup_db().await;
    let now = Utc::now();
    let id =
        insert_scheduled_send(&pool, user_id, "draft-future", now + Duration::minutes(10)).await;
    let submitter = FakeSendSubmitter::default();

    let sent = process_due_scheduled_sends(&pool, &submitter, now)
        .await
        .expect("process due");

    assert_eq!(sent, 0);
    assert!(submitter.calls().is_empty());
    assert_eq!(
        scheduled_send_state(&pool, id).await,
        ("pending".to_string(), None, None, None)
    );
}

#[tokio::test]
async fn transient_failure_leaves_pending() {
    let (pool, _guard, user_id) = setup_db().await;
    let now = Utc::now();
    let id = insert_scheduled_send(
        &pool,
        user_id,
        "draft-transient",
        now - Duration::minutes(1),
    )
    .await;
    let submitter = FakeSendSubmitter::default();
    submitter.set_result(
        "draft-transient",
        Err(SendSubmitError::transient("server unavailable")),
    );

    let sent = process_due_scheduled_sends(&pool, &submitter, now)
        .await
        .expect("process due");

    assert_eq!(sent, 0);
    assert_eq!(
        submitter.calls(),
        vec![(user_id, "draft-transient".to_string())]
    );
    assert_eq!(
        scheduled_send_state(&pool, id).await,
        ("pending".to_string(), None, None, None)
    );
}

#[tokio::test]
async fn permanent_failure_marks_failed_and_sets_error() {
    let (pool, _guard, user_id) = setup_db().await;
    let now = Utc::now();
    let id = insert_scheduled_send(
        &pool,
        user_id,
        "draft-permanent",
        now - Duration::minutes(1),
    )
    .await;
    let submitter = FakeSendSubmitter::default();
    submitter.set_result(
        "draft-permanent",
        Err(SendSubmitError::permanent("invalid recipients")),
    );

    let sent = process_due_scheduled_sends(&pool, &submitter, now)
        .await
        .expect("process due");

    assert_eq!(sent, 0);
    assert_eq!(
        submitter.calls(),
        vec![(user_id, "draft-permanent".to_string())]
    );
    assert_eq!(
        scheduled_send_state(&pool, id).await,
        (
            "failed".to_string(),
            Some(now.to_rfc3339()),
            None,
            Some("invalid recipients".to_string())
        )
    );
    let event: (String, String) = sqlx::query_as(
        "SELECT event_type, payload_json FROM app_events WHERE user_id = ? ORDER BY id DESC LIMIT 1",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .expect("app event");
    assert_eq!(event.0, "send.failed");
    let event_payload: serde_json::Value = serde_json::from_str(&event.1).expect("event payload");
    assert_eq!(event_payload["scheduled_send_id"], id);
    assert_eq!(event_payload["draft_email_id"], "draft-permanent");
    assert_eq!(event_payload["error"], "invalid recipients");

    let audit: (String, String) = sqlx::query_as(
        "SELECT action, payload_json FROM audit_log WHERE user_id = ? ORDER BY id DESC LIMIT 1",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .expect("audit row");
    assert_eq!(audit.0, "compose.send_later.failed");
    let audit_payload: serde_json::Value = serde_json::from_str(&audit.1).expect("audit payload");
    assert_eq!(audit_payload["scheduled_send_id"], id);
    assert_eq!(audit_payload["draft_email_id"], "draft-permanent");
    assert_eq!(audit_payload["error"], "invalid recipients");
}

#[tokio::test]
async fn multiple_due_rows_continue_after_failures() {
    let (pool, _guard, user_id) = setup_db().await;
    let now = Utc::now();
    let first =
        insert_scheduled_send(&pool, user_id, "draft-first", now - Duration::minutes(4)).await;
    let transient = insert_scheduled_send(
        &pool,
        user_id,
        "draft-transient",
        now - Duration::minutes(3),
    )
    .await;
    let permanent = insert_scheduled_send(
        &pool,
        user_id,
        "draft-permanent",
        now - Duration::minutes(2),
    )
    .await;
    let last =
        insert_scheduled_send(&pool, user_id, "draft-last", now - Duration::minutes(1)).await;
    let submitter = FakeSendSubmitter::default();
    submitter.set_result(
        "draft-transient",
        Err(SendSubmitError::transient("rate limited")),
    );
    submitter.set_result(
        "draft-permanent",
        Err(SendSubmitError::permanent("invalid draft")),
    );

    let sent = process_due_scheduled_sends(&pool, &submitter, now)
        .await
        .expect("process due");

    assert_eq!(sent, 2);
    assert_eq!(
        submitter.calls(),
        vec![
            (user_id, "draft-first".to_string()),
            (user_id, "draft-transient".to_string()),
            (user_id, "draft-permanent".to_string()),
            (user_id, "draft-last".to_string()),
        ]
    );
    assert_eq!(scheduled_send_state(&pool, first).await.0, "sent");
    assert_eq!(
        scheduled_send_state(&pool, transient).await,
        ("pending".to_string(), None, None, None)
    );
    assert_eq!(
        scheduled_send_state(&pool, permanent).await,
        (
            "failed".to_string(),
            Some(now.to_rfc3339()),
            None,
            Some("invalid draft".to_string())
        )
    );
    assert_eq!(scheduled_send_state(&pool, last).await.0, "sent");
}

#[tokio::test]
async fn non_pending_due_rows_are_ignored() {
    let (pool, _guard, user_id) = setup_db().await;
    let now = Utc::now();
    let due = now - Duration::minutes(1);
    let sent_id =
        insert_scheduled_send_with_status(&pool, user_id, "draft-sent", due, "sent").await;
    let failed_id =
        insert_scheduled_send_with_status(&pool, user_id, "draft-failed", due, "failed").await;
    let cancelled_id =
        insert_scheduled_send_with_status(&pool, user_id, "draft-cancelled", due, "cancelled")
            .await;
    let processing_id =
        insert_scheduled_send_with_status(&pool, user_id, "draft-processing", due, "processing")
            .await;
    set_scheduled_send_claimed_at(&pool, processing_id, now).await;
    let submitter = FakeSendSubmitter::default();

    let sent = process_due_scheduled_sends(&pool, &submitter, now)
        .await
        .expect("process due");

    assert_eq!(sent, 0);
    assert!(submitter.calls().is_empty());
    assert_eq!(scheduled_send_state(&pool, sent_id).await.0, "sent");
    assert_eq!(scheduled_send_state(&pool, failed_id).await.0, "failed");
    assert_eq!(
        scheduled_send_state(&pool, cancelled_id).await.0,
        "cancelled"
    );
    assert_eq!(
        scheduled_send_state(&pool, processing_id).await.0,
        "processing"
    );
}

#[tokio::test]
async fn duplicate_pending_rows_for_same_draft_are_claimed_independently() {
    let (pool, _guard, user_id) = setup_db().await;
    let now = Utc::now();
    let first = insert_scheduled_send(
        &pool,
        user_id,
        "draft-duplicate",
        now - Duration::minutes(2),
    )
    .await;
    let second = insert_scheduled_send(
        &pool,
        user_id,
        "draft-duplicate",
        now - Duration::minutes(1),
    )
    .await;
    let submitter = FakeSendSubmitter::default();

    let sent = process_due_scheduled_sends(&pool, &submitter, now)
        .await
        .expect("process due");

    assert_eq!(sent, 2);
    assert_eq!(
        submitter.calls(),
        vec![
            (user_id, "draft-duplicate".to_string()),
            (user_id, "draft-duplicate".to_string()),
        ]
    );
    assert_eq!(scheduled_send_state(&pool, first).await.0, "sent");
    assert_eq!(scheduled_send_state(&pool, second).await.0, "sent");
}

#[tokio::test]
async fn stale_processing_claim_fails_unknown_state_without_submit() {
    let (pool, _guard, user_id) = setup_db().await;
    let now = Utc::now();
    let id = insert_scheduled_send_with_status(
        &pool,
        user_id,
        "draft-stale-processing",
        now - Duration::minutes(10),
        "processing",
    )
    .await;
    set_scheduled_send_claimed_at(&pool, id, now - Duration::hours(2)).await;
    let submitter = FakeSendSubmitter::default();

    let sent = process_due_scheduled_sends(&pool, &submitter, now)
        .await
        .expect("process due");

    assert_eq!(sent, 0);
    assert!(submitter.calls().is_empty());
    let state = scheduled_send_state(&pool, id).await;
    assert_eq!(state.0, "failed");
    assert_eq!(state.1, Some((now - Duration::hours(2)).to_rfc3339()));
    assert_eq!(state.2, None);
    assert!(
        state
            .3
            .as_deref()
            .is_some_and(|error| error.contains("submission state unknown")),
        "unexpected error: {:?}",
        state.3
    );

    let event: (String, String) = sqlx::query_as(
        "SELECT event_type, payload_json FROM app_events WHERE user_id = ? ORDER BY id DESC LIMIT 1",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .expect("app event");
    assert_eq!(event.0, "send.failed");
    let event_payload: serde_json::Value = serde_json::from_str(&event.1).expect("event payload");
    assert_eq!(event_payload["scheduled_send_id"], id);
    assert_eq!(event_payload["draft_email_id"], "draft-stale-processing");
    assert!(
        event_payload["error"]
            .as_str()
            .is_some_and(|error| error.contains("submission state unknown")),
        "unexpected event payload: {event_payload}"
    );
    let audit_action: String = sqlx::query_scalar(
        "SELECT action FROM audit_log WHERE user_id = ? ORDER BY id DESC LIMIT 1",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .expect("audit row");
    assert_eq!(audit_action, "compose.send_later.failed");
}

#[tokio::test]
async fn processing_claim_without_claimed_at_fails_unknown_state_without_submit() {
    let (pool, _guard, user_id) = setup_db().await;
    let now = Utc::now();
    let id = insert_scheduled_send_with_status(
        &pool,
        user_id,
        "draft-missing-claim-time",
        now - Duration::minutes(10),
        "processing",
    )
    .await;
    set_scheduled_send_claimed_at_null(&pool, id).await;
    let submitter = FakeSendSubmitter::default();

    let sent = process_due_scheduled_sends(&pool, &submitter, now)
        .await
        .expect("process due");

    assert_eq!(sent, 0);
    assert!(submitter.calls().is_empty());
    let state = scheduled_send_state(&pool, id).await;
    assert_eq!(state.0, "failed");
    assert_eq!(state.1, None);
    assert_eq!(state.2, None);
    assert!(
        state
            .3
            .as_deref()
            .is_some_and(|error| error.contains("submission state unknown")),
        "unexpected error: {:?}",
        state.3
    );
}

#[tokio::test]
async fn recent_processing_claim_is_not_recovered_or_submitted() {
    let (pool, _guard, user_id) = setup_db().await;
    let now = Utc::now();
    let id = insert_scheduled_send_with_status(
        &pool,
        user_id,
        "draft-recent-processing",
        now - Duration::minutes(10),
        "processing",
    )
    .await;
    let claimed_at = now - Duration::minutes(30);
    set_scheduled_send_claimed_at(&pool, id, claimed_at).await;
    let submitter = FakeSendSubmitter::default();

    let sent = process_due_scheduled_sends(&pool, &submitter, now)
        .await
        .expect("process due");

    assert_eq!(sent, 0);
    assert!(submitter.calls().is_empty());
    assert_eq!(
        scheduled_send_state(&pool, id).await,
        (
            "processing".to_string(),
            Some(claimed_at.to_rfc3339()),
            None,
            None
        )
    );
}

#[tokio::test]
async fn stale_claim_recovery_does_not_block_later_due_pending_rows() {
    let (pool, _guard, user_id) = setup_db().await;
    let now = Utc::now();
    let stale = insert_scheduled_send_with_status(
        &pool,
        user_id,
        "draft-stale",
        now - Duration::minutes(10),
        "processing",
    )
    .await;
    set_scheduled_send_claimed_at(&pool, stale, now - Duration::hours(2)).await;
    let pending =
        insert_scheduled_send(&pool, user_id, "draft-pending", now - Duration::minutes(1)).await;
    let submitter = FakeSendSubmitter::default();

    let sent = process_due_scheduled_sends(&pool, &submitter, now)
        .await
        .expect("process due");

    assert_eq!(sent, 1);
    assert_eq!(
        submitter.calls(),
        vec![(user_id, "draft-pending".to_string())]
    );
    assert_eq!(scheduled_send_state(&pool, stale).await.0, "failed");
    assert_eq!(scheduled_send_state(&pool, pending).await.0, "sent");
}

#[tokio::test]
async fn competing_workers_only_submit_claimed_row_once() {
    let (pool, _guard, user_id) = setup_db().await;
    let now = Utc::now();
    let id = insert_scheduled_send(&pool, user_id, "draft-race", now - Duration::minutes(1)).await;
    let submitter = Arc::new(BlockingSubmitter::new());

    let first_pool = pool.clone();
    let first_submitter = submitter.clone();
    let first = tokio::spawn(async move {
        process_due_scheduled_sends(&first_pool, first_submitter.as_ref(), now)
            .await
            .expect("first process due")
    });

    submitter.first_call_entered.wait().await;

    let second_sent = process_due_scheduled_sends(&pool, submitter.as_ref(), now)
        .await
        .expect("second process due");

    submitter.release_first_call.wait().await;
    let first_sent = first.await.expect("first worker join");

    assert_eq!(first_sent, 1);
    assert_eq!(second_sent, 0);
    assert_eq!(submitter.calls(), vec![(user_id, "draft-race".to_string())]);
    assert_eq!(
        scheduled_send_state(&pool, id).await,
        (
            "sent".to_string(),
            Some(now.to_rfc3339()),
            Some(now.to_rfc3339()),
            None
        )
    );
}
