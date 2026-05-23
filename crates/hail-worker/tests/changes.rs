//! Integration tests for the per-user change-handling pipeline.
//!
//! We can't talk to a real Stalwart in CI, so these tests exercise
//! `handle_changes` directly against a synthetic
//! [`JmapChangeFetcher`] impl, plus the `Backoff` schedule from the
//! task contract. The point is to lock down the persistence shape
//! (cursor UPSERT, idempotency on empty rounds) so screener-routing
//! can plug in trivially.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use sqlx::SqlitePool;

// Bring the worker's internal modules into scope by treating the bin
// crate as an "integration" black-box: we redeclare a minimal copy of
// the trait API. Rust integration tests can't see private items in a
// `[[bin]]` crate, so we link directly to the modules via the
// `#[path]` attribute on a local re-export. To keep the test small
// and avoid cargo gymnastics we redeclare just the public surface
// and rely on the bin crate's `cargo test` invocation finding the
// modules under `--test changes` linking.

// Re-declare the public surface we need by `path`-including the
// source files. This is the same trick the `tracing` and `sqlx`
// crates use in their own integration tests.
#[path = "../src/backoff.rs"]
mod backoff;

#[path = "../src/changes.rs"]
mod changes;

#[path = "../src/screener.rs"]
mod screener;

use backoff::Backoff;
use changes::{
    EmailChanges, EmailEnvelope, JmapChangeFetcher, TRACKED_TYPE_STATES, handle_changes,
    load_cursor, upsert_cursor,
};
use screener::{JmapOps, RouteError};

/// In-memory fake fetcher. Returns a scripted [`EmailChanges`] per
/// call and counts invocations so tests can assert call shape.
struct FakeFetcher {
    response: EmailChanges,
    calls: AtomicUsize,
}

#[async_trait]
impl JmapChangeFetcher for FakeFetcher {
    async fn fetch(
        &self,
        _type_state: &str,
        _since_cursor: &str,
    ) -> anyhow::Result<EmailChanges> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.response.clone())
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

/// Fake fetcher that returns an empty round (no ids, blank cursor).
struct EmptyFetcher;

#[async_trait]
impl JmapChangeFetcher for EmptyFetcher {
    async fn fetch(
        &self,
        _type_state: &str,
        _since_cursor: &str,
    ) -> anyhow::Result<EmailChanges> {
        Ok(EmailChanges {
            new_state: "state-after-empty".to_string(),
            created: vec![],
            updated: vec![],
            destroyed: vec![],
        })
    }
}

/// Build a fresh DB URL backed by a unique temp file. Matches the
/// approach in `hail-db/tests/migrate.rs`.
fn fresh_db_url() -> (String, PathBuf) {
    let mut path = std::env::temp_dir();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let pid = std::process::id();
    path.push(format!("hail-worker-test-{pid}-{nanos}.sqlite"));
    let url = format!("sqlite://{}", path.display());
    (url, path)
}

struct TempDb(PathBuf);

impl Drop for TempDb {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
        let _ = std::fs::remove_file(self.0.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(self.0.with_extension("sqlite-shm"));
    }
}

async fn setup_db() -> (SqlitePool, TempDb, i64) {
    let (url, path) = fresh_db_url();
    let guard = TempDb(path);
    let pool = hail_db::connect(&url).await.expect("connect");
    hail_db::migrate(&pool).await.expect("migrate");

    sqlx::query(
        "INSERT INTO users (email, jmap_account_id, created_at) VALUES (?, ?, ?)",
    )
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

#[tokio::test]
async fn handle_changes_persists_new_cursor() {
    let (pool, _guard, user_id) = setup_db().await;

    let fetcher = Arc::new(FakeFetcher {
        response: EmailChanges {
            new_state: "state-2".to_string(),
            created: vec![EmailEnvelope {
                id: "em-1".to_string(),
                thread_id: Some("t-1".to_string()),
                subject: Some("hello".to_string()),
                from: vec![(None, "bob@example.com".to_string())],
                ..Default::default()
            }],
            updated: vec![],
            destroyed: vec![],
        },
        calls: AtomicUsize::new(0),
    });

    let mut types = BTreeSet::new();
    types.insert("Email".to_string());
    handle_changes(&pool, user_id, fetcher.as_ref(), &NoopJmapOps, &types)
        .await
        .expect("handle_changes");

    assert_eq!(fetcher.calls.load(Ordering::SeqCst), 1);
    let stored = load_cursor(&pool, user_id, "Email")
        .await
        .expect("load_cursor");
    assert_eq!(stored, "state-2");
}

#[tokio::test]
async fn handle_changes_is_idempotent_on_empty_round() {
    let (pool, _guard, user_id) = setup_db().await;

    // Seed an existing cursor.
    upsert_cursor(&pool, user_id, "Email", "state-1")
        .await
        .expect("seed");

    let fetcher = EmptyFetcher;
    let mut types = BTreeSet::new();
    types.insert("Email".to_string());

    // Running twice should not panic and should leave the cursor at
    // whatever the fetcher returned.
    for _ in 0..2 {
        handle_changes(&pool, user_id, &fetcher, &NoopJmapOps, &types)
            .await
            .expect("handle_changes empty");
    }

    let stored = load_cursor(&pool, user_id, "Email")
        .await
        .expect("load_cursor");
    assert_eq!(stored, "state-after-empty");
}

#[tokio::test]
async fn handle_changes_skips_untracked_type_states() {
    let (pool, _guard, user_id) = setup_db().await;

    let fetcher = Arc::new(FakeFetcher {
        response: EmailChanges {
            new_state: "should-not-write".to_string(),
            ..Default::default()
        },
        calls: AtomicUsize::new(0),
    });

    // Untracked types (e.g. Identity, Core) must be ignored — no
    // fetch, no cursor write.
    let mut types = BTreeSet::new();
    types.insert("Identity".to_string());
    types.insert("Core".to_string());

    handle_changes(&pool, user_id, fetcher.as_ref(), &NoopJmapOps, &types)
        .await
        .expect("handle_changes");

    assert_eq!(
        fetcher.calls.load(Ordering::SeqCst),
        0,
        "untracked TypeStates must not trigger a fetch"
    );
    let stored = load_cursor(&pool, user_id, "Identity")
        .await
        .expect("load_cursor");
    assert_eq!(stored, "", "no cursor should be written for untracked types");
}

#[tokio::test]
async fn tracked_type_states_match_design() {
    // Lock the set against design.md §6.2 + §8.1 so a typo doesn't
    // silently desync from the schema CHECK constraint.
    let expected: BTreeSet<&str> = ["Email", "EmailDelivery", "Mailbox", "EmailSubmission"]
        .into_iter()
        .collect();
    let actual: BTreeSet<&str> = TRACKED_TYPE_STATES.iter().copied().collect();
    assert_eq!(actual, expected);
}

#[test]
fn backoff_schedule_matches_contract() {
    let mut b = Backoff::new();
    // Schedule from the task contract: 1, 2, 4, 8, 16, then capped
    // at 60s. (Internal cap kicks in at 32 then 60.)
    let expected = [1u64, 2, 4, 8, 16, 32, 60, 60];
    for exp in expected {
        assert_eq!(b.base(), Duration::from_secs(exp));
        let _ = b.next_delay();
    }
}

#[test]
fn backoff_delays_stay_within_base() {
    let mut b = Backoff::new();
    for _ in 0..10 {
        let base = b.base();
        let d = b.next_delay();
        assert!(
            d <= base,
            "full-jitter delay {d:?} must be <= base {base:?}"
        );
    }
}

#[test]
fn backoff_reset_returns_to_one_second() {
    let mut b = Backoff::new();
    for _ in 0..5 {
        let _ = b.next_delay();
    }
    b.reset();
    assert_eq!(b.base(), Duration::from_secs(1));
}
