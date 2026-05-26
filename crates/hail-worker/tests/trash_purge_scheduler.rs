#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use chrono::{Duration, Utc};
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

use scheduler::{TrashPurgeOps, process_trash_purge};

#[derive(Debug, Default)]
struct FakeTrashPurgeOps {
    by_user: Mutex<HashMap<i64, Vec<FakeEmail>>>,
    fail_users: Mutex<HashSet<i64>>,
    calls: Mutex<Vec<i64>>,
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

    fn called_user_ids(&self) -> Vec<i64> {
        self.calls.lock().expect("calls mutex").clone()
    }
}

#[async_trait]
impl TrashPurgeOps for FakeTrashPurgeOps {
    async fn purge_old_trash(&self, user_id: i64, cutoff: chrono::DateTime<Utc>) -> Result<usize> {
        self.calls.lock().expect("calls mutex").push(user_id);

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

async fn setup_db() -> (SqlitePool, TempDb, i64, i64) {
    let (url, guard) = fresh_db_url("hail-worker-trash-purge-scheduler-test");
    let pool = hail_db::connect(&url).await.expect("connect");
    hail_db::migrate(&pool).await.expect("migrate");

    let first = insert_user_with_session(&pool, "alice@example.com").await;
    let second = insert_user_with_session(&pool, "bob@example.com").await;
    (pool, guard, first, second)
}

async fn insert_user_with_session(pool: &SqlitePool, email: &str) -> i64 {
    let now = Utc::now();
    let user_id = insert_user(pool, email, now).await;
    insert_session(
        pool,
        user_id,
        &format!("session-{user_id}"),
        now + Duration::hours(1),
        now,
    )
    .await;
    user_id
}

async fn insert_user(pool: &SqlitePool, email: &str, now: chrono::DateTime<Utc>) -> i64 {
    sqlx::query("INSERT INTO users (email, jmap_account_id, created_at) VALUES (?, ?, ?)")
        .bind(email)
        .bind(format!("acct-{email}"))
        .bind(now.to_rfc3339())
        .execute(pool)
        .await
        .expect("insert user");
    sqlx::query_scalar("SELECT id FROM users WHERE email = ?")
        .bind(email)
        .fetch_one(pool)
        .await
        .expect("fetch user id")
}

async fn insert_session(
    pool: &SqlitePool,
    user_id: i64,
    id: &str,
    expires_at: chrono::DateTime<Utc>,
    now: chrono::DateTime<Utc>,
) {
    sqlx::query(
        "INSERT INTO sessions (id, user_id, jmap_token_enc, expires_at, created_at, last_used_at) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(user_id)
    .bind(vec![1_u8, 2, 3])
    .bind(expires_at.to_rfc3339())
    .bind(now.to_rfc3339())
    .bind(now.to_rfc3339())
    .execute(pool)
    .await
    .expect("insert session");
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
async fn purge_only_runs_for_users_with_active_sessions() {
    let (url, _guard) = fresh_db_url("hail-worker-trash-purge-active-session-filter-test");
    let pool = hail_db::connect(&url).await.expect("connect");
    hail_db::migrate(&pool).await.expect("migrate");

    let now = Utc::now();
    let active_user_id = insert_user(&pool, "active@example.com", now).await;
    insert_session(
        &pool,
        active_user_id,
        "active-session",
        now + Duration::hours(1),
        now,
    )
    .await;

    let expired_user_id = insert_user(&pool, "expired@example.com", now).await;
    insert_session(
        &pool,
        expired_user_id,
        "expired-session",
        now - Duration::seconds(1),
        now,
    )
    .await;

    let no_session_user_id = insert_user(&pool, "no-session@example.com", now).await;

    let jmap = FakeTrashPurgeOps::default();
    jmap.add_email(active_user_id, "active-old", now - Duration::days(31));
    jmap.add_email(expired_user_id, "expired-old", now - Duration::days(31));
    jmap.add_email(
        no_session_user_id,
        "no-session-old",
        now - Duration::days(31),
    );

    let purged = process_trash_purge(&pool, &jmap, 30, now)
        .await
        .expect("process trash purge");

    assert_eq!(purged, 1);
    assert_eq!(jmap.called_user_ids(), vec![active_user_id]);
    assert_eq!(
        jmap.destroyed_ids(active_user_id),
        vec!["active-old".to_string()]
    );
    assert!(jmap.destroyed_ids(expired_user_id).is_empty());
    assert!(jmap.destroyed_ids(no_session_user_id).is_empty());
}

#[tokio::test]
async fn user_with_any_active_session_is_purged_once() {
    let (url, _guard) = fresh_db_url("hail-worker-trash-purge-mixed-sessions-test");
    let pool = hail_db::connect(&url).await.expect("connect");
    hail_db::migrate(&pool).await.expect("migrate");

    let now = Utc::now();
    let user_id = insert_user(&pool, "mixed@example.com", now).await;
    insert_session(
        &pool,
        user_id,
        "mixed-expired-session",
        now - Duration::seconds(1),
        now,
    )
    .await;
    insert_session(
        &pool,
        user_id,
        "mixed-active-session-one",
        now + Duration::hours(1),
        now,
    )
    .await;
    insert_session(
        &pool,
        user_id,
        "mixed-active-session-two",
        now + Duration::hours(2),
        now,
    )
    .await;

    let jmap = FakeTrashPurgeOps::default();
    jmap.add_email(user_id, "old", now - Duration::days(31));

    let purged = process_trash_purge(&pool, &jmap, 30, now)
        .await
        .expect("process trash purge");

    assert_eq!(purged, 1);
    assert_eq!(jmap.called_user_ids(), vec![user_id]);
    assert_eq!(jmap.destroyed_ids(user_id), vec!["old".to_string()]);
}

#[tokio::test]
async fn users_expiring_at_the_purge_tick_are_not_active() {
    let (url, _guard) = fresh_db_url("hail-worker-trash-purge-expiring-now-test");
    let pool = hail_db::connect(&url).await.expect("connect");
    hail_db::migrate(&pool).await.expect("migrate");

    let now = Utc::now();
    let user_id = insert_user(&pool, "boundary@example.com", now).await;
    insert_session(&pool, user_id, "boundary-session", now, now).await;

    let jmap = FakeTrashPurgeOps::default();
    jmap.add_email(user_id, "old", now - Duration::days(31));

    let purged = process_trash_purge(&pool, &jmap, 30, now)
        .await
        .expect("process trash purge");

    assert_eq!(purged, 0);
    assert!(jmap.called_user_ids().is_empty());
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
