#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use sqlx::SqlitePool;

#[path = "../src/crypto.rs"]
#[allow(dead_code)]
mod crypto;

#[path = "../src/reconcile.rs"]
mod reconcile;

use reconcile::{ReconcileReport, ThreadVerifier, process_reconciliation};

#[derive(Debug, Default)]
struct FakeThreadVerifier {
    existing_by_user: HashMap<i64, HashSet<String>>,
}

impl FakeThreadVerifier {
    fn with_existing(mut self, user_id: i64, ids: &[&str]) -> Self {
        self.existing_by_user.insert(
            user_id,
            ids.iter().map(|id| (*id).to_string()).collect(),
        );
        self
    }
}

#[async_trait]
impl ThreadVerifier for FakeThreadVerifier {
    async fn existing_threads(&self, user_id: i64, ids: &[String]) -> Result<HashSet<String>> {
        let allowed = self.existing_by_user.get(&user_id);
        Ok(ids
            .iter()
            .filter(|id| allowed.is_some_and(|set| set.contains(*id)))
            .cloned()
            .collect())
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
    path.push(format!("hail-worker-reconcile-test-{pid}-{nanos}-{counter}.sqlite"));
    let url = format!("sqlite://{}", path.display());
    (url, path)
}

struct TempDb(PathBuf);

impl Drop for TempDb {
    fn drop(&mut self) {
        let _ = &self.0;
    }
}

async fn setup_db() -> (SqlitePool, TempDb, i64, i64) {
    let (url, path) = fresh_db_url();
    let guard = TempDb(path);
    let pool = hail_db::connect(&url).await.expect("connect");
    hail_db::migrate(&pool).await.expect("migrate");

    let alice = insert_user(&pool, "alice@example.com").await;
    let bob = insert_user(&pool, "bob@example.com").await;
    (pool, guard, alice, bob)
}

async fn insert_user(pool: &SqlitePool, email: &str) -> i64 {
    sqlx::query("INSERT INTO users (email, jmap_account_id, created_at) VALUES (?, ?, ?)")
        .bind(email)
        .bind(format!("acct-{email}"))
        .bind("2026-01-01T00:00:00Z")
        .execute(pool)
        .await
        .expect("insert user");
    sqlx::query_scalar("SELECT id FROM users WHERE email = ?")
        .bind(email)
        .fetch_one(pool)
        .await
        .expect("fetch user id")
}

async fn insert_stack(pool: &SqlitePool, user_id: i64, stack: &str, thread_id: &str, position: i64) {
    sqlx::query(
        "INSERT INTO stack_positions (user_id, stack, thread_id, position, added_at) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(user_id)
    .bind(stack)
    .bind(thread_id)
    .bind(position)
    .bind("2026-01-01T00:00:00Z")
    .execute(pool)
    .await
    .expect("insert stack_position");
}

async fn insert_bubble(pool: &SqlitePool, user_id: i64, thread_id: &str, fired_at: Option<&str>) {
    sqlx::query(
        "INSERT INTO bubble_ups (user_id, thread_id, surface_at, fired_at, created_at) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(user_id)
    .bind(thread_id)
    .bind("2026-01-02T00:00:00Z")
    .bind(fired_at)
    .bind("2026-01-01T00:00:00Z")
    .execute(pool)
    .await
    .expect("insert bubble_up");
}

async fn stack_threads(pool: &SqlitePool, user_id: i64) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT thread_id FROM stack_positions WHERE user_id = ? ORDER BY thread_id",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .expect("select stack threads")
}

async fn pending_bubble_threads(pool: &SqlitePool, user_id: i64) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT thread_id FROM bubble_ups \
         WHERE user_id = ? AND fired_at IS NULL ORDER BY thread_id",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .expect("select pending bubble threads")
}

async fn all_bubble_threads(pool: &SqlitePool, user_id: i64) -> Vec<String> {
    sqlx::query_scalar("SELECT thread_id FROM bubble_ups WHERE user_id = ? ORDER BY thread_id")
        .bind(user_id)
        .fetch_all(pool)
        .await
        .expect("select all bubble threads")
}

#[tokio::test]
async fn stack_positions_orphan_removed_existing_kept() {
    let (pool, _guard, alice, _bob) = setup_db().await;
    insert_stack(&pool, alice, "reply_later", "thread-existing", 1).await;
    insert_stack(&pool, alice, "set_aside", "thread-missing", 2).await;
    let verifier = FakeThreadVerifier::default().with_existing(alice, &["thread-existing"]);

    let report = process_reconciliation(&pool, &verifier, Utc::now())
        .await
        .expect("reconcile");

    assert_eq!(stack_threads(&pool, alice).await, vec!["thread-existing"]);
    assert_eq!(
        report,
        ReconcileReport {
            users_checked: 1,
            thread_ids_checked: 2,
            stack_positions_checked: 2,
            stack_positions_deleted: 1,
            bubble_ups_checked: 0,
            bubble_ups_deleted: 0,
        }
    );
}

#[tokio::test]
async fn pending_bubble_up_orphan_removed_existing_kept() {
    let (pool, _guard, alice, _bob) = setup_db().await;
    insert_bubble(&pool, alice, "thread-existing", None).await;
    insert_bubble(&pool, alice, "thread-missing", None).await;
    insert_bubble(&pool, alice, "thread-fired-missing", Some("2026-01-03T00:00:00Z")).await;
    let verifier = FakeThreadVerifier::default().with_existing(alice, &["thread-existing"]);

    let report = process_reconciliation(&pool, &verifier, Utc::now())
        .await
        .expect("reconcile");

    assert_eq!(pending_bubble_threads(&pool, alice).await, vec!["thread-existing"]);
    assert_eq!(
        all_bubble_threads(&pool, alice).await,
        vec!["thread-existing", "thread-fired-missing"]
    );
    assert_eq!(report.bubble_ups_checked, 2);
    assert_eq!(report.bubble_ups_deleted, 1);
}

#[tokio::test]
async fn wrong_user_isolation() {
    let (pool, _guard, alice, bob) = setup_db().await;
    insert_stack(&pool, alice, "reply_later", "shared-thread-id", 1).await;
    insert_stack(&pool, bob, "reply_later", "shared-thread-id", 1).await;
    insert_bubble(&pool, alice, "bubble-shared", None).await;
    insert_bubble(&pool, bob, "bubble-shared", None).await;
    let verifier = FakeThreadVerifier::default()
        .with_existing(alice, &["shared-thread-id", "bubble-shared"]);

    let report = process_reconciliation(&pool, &verifier, Utc::now())
        .await
        .expect("reconcile");

    assert_eq!(stack_threads(&pool, alice).await, vec!["shared-thread-id"]);
    assert!(stack_threads(&pool, bob).await.is_empty());
    assert_eq!(pending_bubble_threads(&pool, alice).await, vec!["bubble-shared"]);
    assert!(pending_bubble_threads(&pool, bob).await.is_empty());
    assert_eq!(report.users_checked, 2);
    assert_eq!(report.stack_positions_deleted, 1);
    assert_eq!(report.bubble_ups_deleted, 1);
}

#[tokio::test]
async fn empty_tables_no_op() {
    let (pool, _guard, _alice, _bob) = setup_db().await;
    let verifier = FakeThreadVerifier::default();

    let report = process_reconciliation(&pool, &verifier, Utc::now())
        .await
        .expect("reconcile");

    assert_eq!(report, ReconcileReport::default());
}
