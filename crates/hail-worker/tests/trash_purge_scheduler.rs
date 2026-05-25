#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use chrono::{Duration, Utc};
use sqlx::SqlitePool;

#[path = "../src/app_events.rs"]
mod app_events;

#[path = "../src/crypto.rs"]
#[allow(dead_code)]
mod crypto;

#[path = "../src/scheduler.rs"]
mod scheduler;

use scheduler::{TrashPurgeOps, process_trash_purge};

#[derive(Debug, Default)]
struct FakeTrashPurgeOps {
    by_user: Mutex<HashMap<i64, Vec<FakeEmail>>>,
    fail_users: Mutex<HashSet<i64>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FakeEmail {
    id: String,
    received_at: chrono::DateTime<Utc>,
    destroyed: bool,
}

impl FakeTrashPurgeOps {
    fn add_email(&self, user_id: i64, id: &str, received_at: chrono::DateTime<Utc>) {
        self.by_user
            .lock()
            .expect("by_user mutex")
            .entry(user_id)
            .or_default()
            .push(FakeEmail {
                id: id.to_string(),
                received_at,
                destroyed: false,
            });
    }

    fn fail_user(&self, user_id: i64) {
        self.fail_users
            .lock()
            .expect("fail_users mutex")
            .insert(user_id);
    }

    fn destroyed_ids(&self, user_id: i64) -> Vec<String> {
        self.by_user
            .lock()
            .expect("by_user mutex")
            .get(&user_id)
            .into_iter()
            .flatten()
            .filter(|email| email.destroyed)
            .map(|email| email.id.clone())
            .collect()
    }
}

#[async_trait]
impl TrashPurgeOps for FakeTrashPurgeOps {
    async fn purge_old_trash(&self, user_id: i64, cutoff: chrono::DateTime<Utc>) -> Result<usize> {
        if self
            .fail_users
            .lock()
            .expect("fail_users mutex")
            .contains(&user_id)
        {
            return Err(anyhow!("scripted JMAP failure for user {user_id}"));
        }

        let mut by_user = self.by_user.lock().expect("by_user mutex");
        let purged = by_user
            .entry(user_id)
            .or_default()
            .iter_mut()
            .filter(|email| email.received_at < cutoff)
            .map(|email| {
                email.destroyed = true;
                1_usize
            })
            .sum();
        Ok(purged)
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
        "hail-worker-trash-purge-test-{pid}-{nanos}-{counter}.sqlite"
    ));
    let url = format!("sqlite://{}", path.display());
    (url, path)
}

struct TempDb(PathBuf);

impl Drop for TempDb {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

async fn setup_db() -> (SqlitePool, TempDb, i64, i64) {
    let (url, path) = fresh_db_url();
    let guard = TempDb(path);
    let pool = hail_db::connect(&url).await.expect("connect");
    hail_db::migrate(&pool).await.expect("migrate");

    let first = insert_user_with_session(&pool, "alice@example.com").await;
    let second = insert_user_with_session(&pool, "bob@example.com").await;
    (pool, guard, first, second)
}

async fn insert_user_with_session(pool: &SqlitePool, email: &str) -> i64 {
    let now = Utc::now();
    sqlx::query("INSERT INTO users (email, jmap_account_id, created_at) VALUES (?, ?, ?)")
        .bind(email)
        .bind(format!("acct-{email}"))
        .bind(now.to_rfc3339())
        .execute(pool)
        .await
        .expect("insert user");
    let user_id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE email = ?")
        .bind(email)
        .fetch_one(pool)
        .await
        .expect("fetch user id");
    sqlx::query(
        "INSERT INTO sessions (id, user_id, jmap_token_enc, expires_at, created_at, last_used_at) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(format!("session-{user_id}"))
    .bind(user_id)
    .bind(vec![1_u8, 2, 3])
    .bind((now + Duration::hours(1)).to_rfc3339())
    .bind(now.to_rfc3339())
    .bind(now.to_rfc3339())
    .execute(pool)
    .await
    .expect("insert session");
    user_id
}

#[tokio::test]
async fn emails_older_than_retention_are_destroyed() {
    let (pool, _guard, user_id, _other_user_id) = setup_db().await;
    let now = Utc::now();
    let jmap = FakeTrashPurgeOps::default();
    jmap.add_email(user_id, "old", now - Duration::days(31));
    jmap.add_email(user_id, "borderline-recent", now - Duration::days(29));

    let purged = process_trash_purge(&pool, &jmap, 30, now)
        .await
        .expect("process trash purge");

    assert_eq!(purged, 1);
    assert_eq!(jmap.destroyed_ids(user_id), vec!["old".to_string()]);
}

#[tokio::test]
async fn recent_emails_are_kept() {
    let (pool, _guard, user_id, _other_user_id) = setup_db().await;
    let now = Utc::now();
    let jmap = FakeTrashPurgeOps::default();
    jmap.add_email(user_id, "recent", now - Duration::days(3));

    let purged = process_trash_purge(&pool, &jmap, 30, now)
        .await
        .expect("process trash purge");

    assert_eq!(purged, 0);
    assert!(jmap.destroyed_ids(user_id).is_empty());
}

#[tokio::test]
async fn jmap_failure_for_one_user_is_logged_and_later_users_continue() {
    let (pool, _guard, failing_user_id, succeeding_user_id) = setup_db().await;
    let now = Utc::now();
    let jmap = FakeTrashPurgeOps::default();
    jmap.fail_user(failing_user_id);
    jmap.add_email(
        failing_user_id,
        "not-destroyed-after-failure",
        now - Duration::days(31),
    );
    jmap.add_email(
        succeeding_user_id,
        "destroyed-for-later-user",
        now - Duration::days(31),
    );

    let purged = process_trash_purge(&pool, &jmap, 30, now)
        .await
        .expect("process trash purge");

    assert_eq!(purged, 1);
    assert!(jmap.destroyed_ids(failing_user_id).is_empty());
    assert_eq!(
        jmap.destroyed_ids(succeeding_user_id),
        vec!["destroyed-for-later-user".to_string()]
    );
}
