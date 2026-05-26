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

use scheduler::{SpamPurgeOps, process_spam_purge};

#[derive(Debug, Default)]
struct FakeSpamPurgeOps {
    by_user: Mutex<HashMap<i64, Vec<FakeEmail>>>,
    fail_users: Mutex<HashSet<i64>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FakeEmail {
    id: String,
    received_at: chrono::DateTime<Utc>,
    in_junk_mailbox: bool,
    has_junk_keyword: bool,
    destroyed: bool,
}

impl FakeSpamPurgeOps {
    fn add_email(
        &self,
        user_id: i64,
        id: &str,
        received_at: chrono::DateTime<Utc>,
        in_junk_mailbox: bool,
        has_junk_keyword: bool,
    ) {
        self.by_user
            .lock()
            .expect("by_user mutex")
            .entry(user_id)
            .or_default()
            .push(FakeEmail {
                id: id.to_string(),
                received_at,
                in_junk_mailbox,
                has_junk_keyword,
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
impl SpamPurgeOps for FakeSpamPurgeOps {
    async fn purge_old_spam(&self, user_id: i64, cutoff: chrono::DateTime<Utc>) -> Result<usize> {
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
            .filter(|email| {
                email.received_at < cutoff && (email.in_junk_mailbox || email.has_junk_keyword)
            })
            .map(|email| {
                email.destroyed = true;
                1_usize
            })
            .sum();
        Ok(purged)
    }
}

async fn setup_db() -> (SqlitePool, TempDb, i64, i64) {
    let (url, guard) = fresh_db_url("hail-worker-spam-purge-scheduler-test");
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
async fn emails_older_than_seven_days_are_destroyed_from_junk_mailbox_or_keyword() {
    let (pool, _guard, user_id, _other_user_id) = setup_db().await;
    let now = Utc::now();
    let jmap = FakeSpamPurgeOps::default();
    jmap.add_email(user_id, "old-mailbox", now - Duration::days(8), true, false);
    jmap.add_email(user_id, "old-keyword", now - Duration::days(8), false, true);
    jmap.add_email(user_id, "recent-mailbox", now - Duration::days(6), true, false);
    jmap.add_email(user_id, "old-not-spam", now - Duration::days(8), false, false);

    let purged = process_spam_purge(&pool, &jmap, now)
        .await
        .expect("process spam purge");

    assert_eq!(purged, 2);
    assert_eq!(
        jmap.destroyed_ids(user_id),
        vec!["old-mailbox".to_string(), "old-keyword".to_string()]
    );
}

#[tokio::test]
async fn jmap_failure_for_one_user_is_logged_and_later_users_continue() {
    let (pool, _guard, failing_user_id, succeeding_user_id) = setup_db().await;
    let now = Utc::now();
    let jmap = FakeSpamPurgeOps::default();
    jmap.fail_user(failing_user_id);
    jmap.add_email(
        failing_user_id,
        "not-destroyed-after-failure",
        now - Duration::days(8),
        true,
        false,
    );
    jmap.add_email(
        succeeding_user_id,
        "destroyed-for-later-user",
        now - Duration::days(8),
        false,
        true,
    );

    let purged = process_spam_purge(&pool, &jmap, now)
        .await
        .expect("process spam purge");

    assert_eq!(purged, 1);
    assert!(jmap.destroyed_ids(failing_user_id).is_empty());
    assert_eq!(
        jmap.destroyed_ids(succeeding_user_id),
        vec!["destroyed-for-later-user".to_string()]
    );
}
