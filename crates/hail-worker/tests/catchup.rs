//! Integration tests for startup/reconnect catch-up behavior.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use sqlx::SqlitePool;
use tokio_util::sync::CancellationToken;

#[path = "../src/app_events.rs"]
mod app_events;
#[path = "../src/catchup.rs"]
mod catchup;
#[path = "../src/changes.rs"]
mod changes;
#[path = "../src/screener.rs"]
mod screener;

use catchup::catchup_user;
use changes::{
    EmailChanges, EmailEnvelope, JmapChangeFetcher, TRACKED_TYPE_STATES, load_cursor, upsert_cursor,
};
use screener::{JmapOps, RouteError};

#[derive(Clone)]
struct ScriptedResponse {
    current_state: String,
    changes: EmailChanges,
}

struct FakeFetcher {
    by_type: HashMap<String, ScriptedResponse>,
    current_state_calls: AtomicUsize,
    fetch_calls: AtomicUsize,
    gate: Option<Arc<tokio::sync::Notify>>,
}

#[async_trait]
impl JmapChangeFetcher for FakeFetcher {
    async fn current_state(&self, type_state: &str) -> anyhow::Result<String> {
        self.current_state_calls.fetch_add(1, Ordering::SeqCst);
        if let Some(gate) = &self.gate {
            gate.notified().await;
        }
        Ok(self
            .by_type
            .get(type_state)
            .map(|r| r.current_state.clone())
            .unwrap_or_else(|| format!("{type_state}-current")))
    }

    async fn fetch(&self, type_state: &str, since_cursor: &str) -> anyhow::Result<EmailChanges> {
        self.fetch_calls.fetch_add(1, Ordering::SeqCst);
        let mut changes = self
            .by_type
            .get(type_state)
            .map(|r| r.changes.clone())
            .unwrap_or_default();
        if changes.new_state.is_empty() {
            changes.new_state = since_cursor.to_string();
        }
        Ok(changes)
    }
}

/// Fetcher whose `current_state` blocks forever. The catch-up future
/// should still exit promptly when the caller cancels/drops it.
struct BlockingFetcher {
    entered: tokio::sync::Notify,
}

#[async_trait]
impl JmapChangeFetcher for BlockingFetcher {
    async fn current_state(&self, _type_state: &str) -> anyhow::Result<String> {
        self.entered.notify_waiters();
        std::future::pending::<()>().await;
        unreachable!()
    }

    async fn fetch(&self, _type_state: &str, _since_cursor: &str) -> anyhow::Result<EmailChanges> {
        self.entered.notify_waiters();
        std::future::pending::<()>().await;
        unreachable!()
    }
}

/// Fetcher that fails every replay fetch. Used to ensure catch-up surfaces
/// persisted-cursor replay errors instead of silently continuing to EventSource.
struct FailingFetchFetcher {
    calls: AtomicUsize,
}

#[async_trait]
impl JmapChangeFetcher for FailingFetchFetcher {
    async fn current_state(&self, _type_state: &str) -> anyhow::Result<String> {
        Ok("unused".to_string())
    }

    async fn fetch(&self, _type_state: &str, _since_cursor: &str) -> anyhow::Result<EmailChanges> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        anyhow::bail!("scripted fetch failure")
    }
}

struct NoopJmapOps;

#[async_trait]
impl JmapOps for NoopJmapOps {
    async fn get_or_create_mailbox(&self, _name: &str) -> Result<String, RouteError> {
        Ok("screener-id".to_string())
    }

    async fn get_mailbox_by_role(&self, _role: &str) -> Result<Option<String>, RouteError> {
        Ok(Some("trash-id".to_string()))
    }

    async fn apply_keyword(&self, _email_id: &str, _keyword: &str) -> Result<(), RouteError> {
        Ok(())
    }

    async fn move_to_mailbox(&self, _email_id: &str, _mailbox_id: &str) -> Result<(), RouteError> {
        Ok(())
    }
}

static DB_COUNTER: AtomicU64 = AtomicU64::new(0);

fn fresh_db_url() -> (String, PathBuf) {
    let mut path = std::env::temp_dir();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let pid = std::process::id();
    let counter = DB_COUNTER.fetch_add(1, Ordering::SeqCst);
    path.push(format!(
        "hail-worker-catchup-test-{pid}-{nanos}-{counter}.sqlite"
    ));
    let url = format!("sqlite://{}", path.display());
    (url, path)
}

struct TempDb(PathBuf);

impl Drop for TempDb {
    fn drop(&mut self) {
        // Intentionally leave temp DB files behind. Integration tests
        // bind the pool before the guard, so removing the files here
        // can race SQLite's pool close under parallel test execution.
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

fn fake_with(changes: EmailChanges) -> FakeFetcher {
    let mut by_type = HashMap::new();
    for type_state in TRACKED_TYPE_STATES {
        by_type.insert(
            (*type_state).to_string(),
            ScriptedResponse {
                current_state: format!("{type_state}-current"),
                changes: changes.clone(),
            },
        );
    }
    FakeFetcher {
        by_type,
        current_state_calls: AtomicUsize::new(0),
        fetch_calls: AtomicUsize::new(0),
        gate: None,
    }
}

#[tokio::test]
async fn empty_cursor_fetches_current_state_and_persists_zero_changes() {
    let (pool, _guard, user_id) = setup_db().await;
    let fetcher = fake_with(EmailChanges::default());

    catchup_user(
        &pool,
        user_id,
        &fetcher,
        &NoopJmapOps,
        CancellationToken::new(),
    )
    .await
    .expect("catchup");

    assert_eq!(fetcher.current_state_calls.load(Ordering::SeqCst), 4);
    assert_eq!(fetcher.fetch_calls.load(Ordering::SeqCst), 0);
    for type_state in TRACKED_TYPE_STATES {
        let stored = load_cursor(&pool, user_id, type_state)
            .await
            .expect("cursor");
        assert_eq!(stored, format!("{type_state}-current"));
    }
}

#[tokio::test]
async fn stored_cursor_matches_server_empty_diff_no_work() {
    let (pool, _guard, user_id) = setup_db().await;
    for type_state in TRACKED_TYPE_STATES {
        upsert_cursor(&pool, user_id, type_state, "state-1")
            .await
            .expect("seed");
    }
    let fetcher = fake_with(EmailChanges {
        new_state: "state-1".to_string(),
        ..Default::default()
    });

    catchup_user(
        &pool,
        user_id,
        &fetcher,
        &NoopJmapOps,
        CancellationToken::new(),
    )
    .await
    .expect("catchup");

    assert_eq!(fetcher.current_state_calls.load(Ordering::SeqCst), 0);
    assert_eq!(fetcher.fetch_calls.load(Ordering::SeqCst), 4);
    for type_state in TRACKED_TYPE_STATES {
        let stored = load_cursor(&pool, user_id, type_state)
            .await
            .expect("cursor");
        assert_eq!(stored, "state-1");
    }
}

#[tokio::test]
async fn stored_cursor_fetch_failure_aborts_catchup_without_advancing_cursor() {
    let (pool, _guard, user_id) = setup_db().await;
    for type_state in TRACKED_TYPE_STATES {
        upsert_cursor(&pool, user_id, type_state, "state-before-error")
            .await
            .expect("seed");
    }
    let fetcher = FailingFetchFetcher {
        calls: AtomicUsize::new(0),
    };

    let err = catchup_user(
        &pool,
        user_id,
        &fetcher,
        &NoopJmapOps,
        CancellationToken::new(),
    )
    .await
    .expect_err("catchup must surface replay fetch failures");

    assert_eq!(fetcher.calls.load(Ordering::SeqCst), 1);
    assert!(
        err.to_string()
            .contains("fetch Email changes during catchup"),
        "unexpected error context: {err:#}"
    );
    for type_state in TRACKED_TYPE_STATES {
        let stored = load_cursor(&pool, user_id, type_state)
            .await
            .expect("cursor");
        assert_eq!(stored, "state-before-error");
    }
}

#[tokio::test]
async fn stored_cursor_lagging_applies_n_envelopes_and_advances_cursor() {
    let (pool, _guard, user_id) = setup_db().await;
    upsert_cursor(&pool, user_id, "Email", "state-old")
        .await
        .expect("seed");

    let mut by_type = HashMap::new();
    by_type.insert(
        "Email".to_string(),
        ScriptedResponse {
            current_state: "unused".to_string(),
            changes: EmailChanges {
                new_state: "state-new".to_string(),
                created: vec![
                    EmailEnvelope {
                        id: "e1".to_string(),
                        ..Default::default()
                    },
                    EmailEnvelope {
                        id: "e2".to_string(),
                        ..Default::default()
                    },
                ],
                updated: vec![EmailEnvelope {
                    id: "e3".to_string(),
                    ..Default::default()
                }],
                destroyed: vec![],
            },
        },
    );
    for type_state in ["EmailDelivery", "Mailbox", "EmailSubmission"] {
        by_type.insert(
            type_state.to_string(),
            ScriptedResponse {
                current_state: format!("{type_state}-current"),
                changes: EmailChanges::default(),
            },
        );
    }
    let fetcher = FakeFetcher {
        by_type,
        current_state_calls: AtomicUsize::new(0),
        fetch_calls: AtomicUsize::new(0),
        gate: None,
    };

    catchup_user(
        &pool,
        user_id,
        &fetcher,
        &NoopJmapOps,
        CancellationToken::new(),
    )
    .await
    .expect("catchup");

    assert_eq!(fetcher.fetch_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fetcher.current_state_calls.load(Ordering::SeqCst), 3);
    let stored = load_cursor(&pool, user_id, "Email").await.expect("cursor");
    assert_eq!(stored, "state-new");
}

#[tokio::test]
async fn cancel_during_catchup_exits_within_100ms() {
    let (pool, _guard, user_id) = setup_db().await;
    let cancel = CancellationToken::new();
    let fetcher = Arc::new(BlockingFetcher {
        entered: tokio::sync::Notify::new(),
    });

    let handle = tokio::spawn({
        let pool = pool.clone();
        let fetcher = Arc::clone(&fetcher);
        let cancel = cancel.clone();
        async move {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => Ok(()),
                result = catchup_user(&pool, user_id, fetcher.as_ref(), &NoopJmapOps, cancel.clone()) => result,
            }
        }
    });
    fetcher.entered.notified().await;
    cancel.cancel();

    let result = tokio::time::timeout(Duration::from_millis(100), handle)
        .await
        .expect("catchup exits within 100ms")
        .expect("join");
    result.expect("catchup result");
}
