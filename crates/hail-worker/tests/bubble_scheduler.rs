#![allow(dead_code)]

use std::collections::HashSet;
use std::sync::Mutex;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use hail_test::{TempDb, fresh_db_url};
use sqlx::SqlitePool;

#[path = "../src/app_events.rs"]
mod app_events;

#[path = "../src/crypto.rs"]
#[allow(dead_code)]
mod crypto;

#[path = "../src/jmap_helpers.rs"]
mod jmap_helpers;

#[path = "../src/scheduler.rs"]
mod scheduler;

use scheduler::{BubbleJmapOps, process_due_bubble_ups};

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResurfaceCall {
    user_id: i64,
    thread_id: String,
    set_keywords: Vec<String>,
    removed_keywords: Vec<String>,
}

#[derive(Debug, Default)]
struct FakeBubbleJmapOps {
    resurfaces: Mutex<Vec<ResurfaceCall>>,
    fail_threads: Mutex<HashSet<String>>,
}

impl FakeBubbleJmapOps {
    fn fail_thread(&self, thread_id: &str) {
        self.fail_threads
            .lock()
            .expect("fail_threads mutex")
            .insert(thread_id.to_string());
    }

    fn resurfaces(&self) -> Vec<ResurfaceCall> {
        self.resurfaces.lock().expect("resurfaces mutex").clone()
    }
}

#[async_trait]
impl BubbleJmapOps for FakeBubbleJmapOps {
    async fn resurface_thread(&self, user_id: i64, thread_id: &str) -> Result<()> {
        self.resurfaces
            .lock()
            .expect("resurfaces mutex")
            .push(ResurfaceCall {
                user_id,
                thread_id: thread_id.to_string(),
                set_keywords: vec!["$hail_imbox".to_string()],
                removed_keywords: vec![
                    "$seen".to_string(),
                    "$hail_setaside".to_string(),
                    "$hail_replylater".to_string(),
                    "$hail_feed".to_string(),
                    "$hail_papertrail".to_string(),
                ],
            });
        if self
            .fail_threads
            .lock()
            .expect("fail_threads mutex")
            .contains(thread_id)
        {
            Err(anyhow!("scripted JMAP failure for {thread_id}"))
        } else {
            Ok(())
        }
    }
}

async fn setup_db() -> (SqlitePool, TempDb, i64) {
    let (url, guard) = fresh_db_url("hail-worker-bubble-scheduler-test");
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

async fn insert_bubble_up(
    pool: &SqlitePool,
    user_id: i64,
    thread_id: &str,
    surface_at: DateTime<Utc>,
) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO bubble_ups (user_id, thread_id, surface_at, created_at) \
         VALUES (?, ?, ?, ?) RETURNING id",
    )
    .bind(user_id)
    .bind(thread_id)
    .bind(surface_at)
    .bind("2026-01-01T00:00:00Z")
    .fetch_one(pool)
    .await
    .expect("insert bubble_up")
}

async fn fired_at(pool: &SqlitePool, id: i64) -> Option<String> {
    sqlx::query_scalar("SELECT fired_at FROM bubble_ups WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("select fired_at")
}

async fn stack_position_count(pool: &SqlitePool, user_id: i64, thread_id: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM stack_positions WHERE user_id = ? AND thread_id = ?")
        .bind(user_id)
        .bind(thread_id)
        .fetch_one(pool)
        .await
        .expect("select stack_position count")
}

async fn insert_stack_position(pool: &SqlitePool, user_id: i64, stack: &str, thread_id: &str) {
    sqlx::query(
        "INSERT INTO stack_positions (user_id, stack, thread_id, position, added_at) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(user_id)
    .bind(stack)
    .bind(thread_id)
    .bind(1_i64)
    .bind("2026-01-01T00:00:00Z")
    .execute(pool)
    .await
    .expect("insert stack_position");
}

#[tokio::test]
async fn due_row_fires_resurfaces_thread_cleans_stack_and_sets_fired_at() {
    let (pool, _guard, user_id) = setup_db().await;
    let now = Utc::now();
    let id = insert_bubble_up(&pool, user_id, "thread-due", now - Duration::minutes(1)).await;
    insert_stack_position(&pool, user_id, "set_aside", "thread-due").await;
    insert_stack_position(&pool, user_id, "reply_later", "thread-due").await;
    let jmap = FakeBubbleJmapOps::default();

    let fired = process_due_bubble_ups(&pool, &jmap, now)
        .await
        .expect("process due");

    assert_eq!(fired, 1);
    assert_eq!(
        jmap.resurfaces(),
        vec![ResurfaceCall {
            user_id,
            thread_id: "thread-due".to_string(),
            set_keywords: vec!["$hail_imbox".to_string()],
            removed_keywords: vec![
                "$seen".to_string(),
                "$hail_setaside".to_string(),
                "$hail_replylater".to_string(),
                "$hail_feed".to_string(),
                "$hail_papertrail".to_string(),
            ],
        }]
    );
    assert_eq!(stack_position_count(&pool, user_id, "thread-due").await, 0);
    assert_eq!(
        fired_at(&pool, id).await.as_deref(),
        Some(now.to_rfc3339().as_str())
    );
    let event_type: String = sqlx::query_scalar(
        "SELECT event_type FROM app_events WHERE user_id = ? ORDER BY id DESC LIMIT 1",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .expect("app event");
    assert_eq!(event_type, "bubble.fired");
}

#[tokio::test]
async fn future_row_not_touched() {
    let (pool, _guard, user_id) = setup_db().await;
    let now = Utc::now();
    let id = insert_bubble_up(&pool, user_id, "thread-future", now + Duration::minutes(10)).await;
    let jmap = FakeBubbleJmapOps::default();

    let fired = process_due_bubble_ups(&pool, &jmap, now)
        .await
        .expect("process due");

    assert_eq!(fired, 0);
    assert!(jmap.resurfaces().is_empty());
    assert!(fired_at(&pool, id).await.is_none());
}

#[tokio::test]
async fn jmap_failure_leaves_row_pending() {
    let (pool, _guard, user_id) = setup_db().await;
    let now = Utc::now();
    let id = insert_bubble_up(&pool, user_id, "thread-fails", now - Duration::minutes(1)).await;
    let jmap = FakeBubbleJmapOps::default();
    jmap.fail_thread("thread-fails");

    let fired = process_due_bubble_ups(&pool, &jmap, now)
        .await
        .expect("process due");

    assert_eq!(fired, 0);
    assert_eq!(
        jmap.resurfaces()
            .into_iter()
            .map(|call| (call.user_id, call.thread_id))
            .collect::<Vec<_>>(),
        vec![(user_id, "thread-fails".to_string())]
    );
    assert!(fired_at(&pool, id).await.is_none());
}

#[tokio::test]
async fn multiple_due_rows_continue_after_one_failure() {
    let (pool, _guard, user_id) = setup_db().await;
    let now = Utc::now();
    let first = insert_bubble_up(&pool, user_id, "thread-first", now - Duration::minutes(3)).await;
    let failed = insert_bubble_up(&pool, user_id, "thread-fails", now - Duration::minutes(2)).await;
    let last = insert_bubble_up(&pool, user_id, "thread-last", now - Duration::minutes(1)).await;
    let jmap = FakeBubbleJmapOps::default();
    jmap.fail_thread("thread-fails");

    let fired = process_due_bubble_ups(&pool, &jmap, now)
        .await
        .expect("process due");

    assert_eq!(fired, 2);
    assert_eq!(
        jmap.resurfaces()
            .into_iter()
            .map(|call| (call.user_id, call.thread_id))
            .collect::<Vec<_>>(),
        vec![
            (user_id, "thread-first".to_string()),
            (user_id, "thread-fails".to_string()),
            (user_id, "thread-last".to_string()),
        ]
    );
    assert!(fired_at(&pool, first).await.is_some());
    assert!(fired_at(&pool, failed).await.is_none());
    assert!(fired_at(&pool, last).await.is_some());
}
