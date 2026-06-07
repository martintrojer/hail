use std::sync::Arc;

use chrono::{Duration, Utc};
use hail_blob_store::{BlobStore, FilesystemBlobStore};
use hail_cache::{CachePolicy, evict_account_bodies, refresh_pinned_messages};
use hail_core::{BlobKind, MailBackfill, MailCacheMode};
use hail_test::{TempDb, fresh_db_url};
use sqlx::SqlitePool;

async fn setup_db(
    name: &str,
) -> (
    SqlitePool,
    TempDb,
    tempfile::TempDir,
    Arc<FilesystemBlobStore>,
) {
    let (url, guard) = fresh_db_url(name);
    let pool = hail_db::connect(&url).await.expect("connect");
    hail_db::migrate(&pool).await.expect("migrate");
    ensure_account(&pool).await;
    let tempdir = tempfile::tempdir().expect("blob tempdir");
    let store = Arc::new(
        FilesystemBlobStore::new(tempdir.path()).with_sweep_grace(std::time::Duration::ZERO),
    );
    (pool, guard, tempdir, store)
}

async fn ensure_account(pool: &SqlitePool) {
    sqlx::query(
        "INSERT INTO users (id, email, jmap_account_id, display_name, is_admin, created_at) \
         VALUES (1, 'cache@example.test', 'acct', NULL, 1, '2026-01-01T00:00:00Z')",
    )
    .execute(pool)
    .await
    .expect("insert user");
    sqlx::query(
        "INSERT INTO mail_accounts \
         (id, user_id, jmap_account_id, backend_kind, provider_kind, provider_account_id, provider_email, refresh_token_enc, sync_status, created_at, updated_at) \
         VALUES (1, 1, 'acct', 'gmail', 'gmail', 'provider-acct', 'cache@example.test', ?1, 'active', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .bind(vec![7_u8; 32])
    .execute(pool)
    .await
    .expect("insert account");
}

fn policy(
    mode: MailCacheMode,
    keep_days: u32,
    keep_max_msgs: u64,
    keep_max_bytes: u64,
) -> CachePolicy {
    CachePolicy::new(
        mode,
        keep_days,
        keep_max_msgs,
        keep_max_bytes,
        MailBackfill::Off,
    )
}

async fn insert_message(
    pool: &SqlitePool,
    store: &dyn BlobStore,
    backend_id: &str,
    accessed_offset_days: i64,
    internal_offset_days: i64,
    size_bytes: i64,
    pinned: bool,
) -> String {
    let blob_id = store
        .put(BlobKind::Eml, format!("body {backend_id}").as_bytes())
        .await
        .expect("put blob")
        .to_string();
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO messages \
         (account_id, backend_msg_id, thread_id, internal_date, from_addr, subject, preview, size_bytes, body_blob_id, body_text, inserted_at, accessed_at, pinned) \
         VALUES (1, ?1, ?2, ?3, 'sender@example.test', ?1, 'preview', ?4, ?5, 'body text', ?6, ?7, ?8)",
    )
    .bind(backend_id)
    .bind(format!("thread-{backend_id}"))
    .bind((now - Duration::days(internal_offset_days)).timestamp())
    .bind(size_bytes)
    .bind(&blob_id)
    .bind(now.to_rfc3339())
    .bind((now - Duration::days(accessed_offset_days)).to_rfc3339())
    .bind(i64::from(pinned))
    .execute(pool)
    .await
    .expect("insert message");
    blob_id
}

async fn body_blob(pool: &SqlitePool, backend_id: &str) -> Option<String> {
    sqlx::query_scalar("SELECT body_blob_id FROM messages WHERE backend_msg_id = ?1")
        .bind(backend_id)
        .fetch_one(pool)
        .await
        .expect("body blob")
}

async fn metadata_count(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM messages")
        .fetch_one(pool)
        .await
        .expect("message count")
}

#[tokio::test]
async fn age_eviction_clears_body_but_keeps_metadata() {
    let (pool, _guard, _tempdir, store) = setup_db("hail-cache-age-eviction").await;
    let old_blob = insert_message(&pool, store.as_ref(), "old", 1, 10, 10, false).await;
    let recent_blob = insert_message(&pool, store.as_ref(), "recent", 1, 1, 10, false).await;

    let stats = evict_account_bodies(
        &pool,
        store.as_ref(),
        1,
        &policy(MailCacheMode::Bounded, 5, 10, 1_000),
    )
    .await
    .expect("evict");

    assert_eq!(stats.evicted_bodies, 1);
    assert_eq!(body_blob(&pool, "old").await, None);
    assert_eq!(
        body_blob(&pool, "recent").await.as_deref(),
        Some(recent_blob.as_str())
    );
    assert_eq!(metadata_count(&pool).await, 2);
    assert!(
        !store
            .exists(&hail_core::BlobId::parse(&old_blob).expect("old blob id"))
            .await
            .expect("old exists")
    );
}

#[tokio::test]
async fn count_eviction_uses_lru_and_keeps_metadata() {
    let (pool, _guard, _tempdir, store) = setup_db("hail-cache-count-eviction").await;
    insert_message(&pool, store.as_ref(), "least-recent", 10, 1, 10, false).await;
    let newest_blob = insert_message(&pool, store.as_ref(), "newest", 1, 1, 10, false).await;

    let stats = evict_account_bodies(
        &pool,
        store.as_ref(),
        1,
        &policy(MailCacheMode::Bounded, 30, 1, 1_000),
    )
    .await
    .expect("evict");

    assert_eq!(stats.evicted_bodies, 1);
    assert_eq!(body_blob(&pool, "least-recent").await, None);
    assert_eq!(
        body_blob(&pool, "newest").await.as_deref(),
        Some(newest_blob.as_str())
    );
    assert_eq!(metadata_count(&pool).await, 2);
}

#[tokio::test]
async fn size_eviction_keeps_newest_accessed_rows_within_budget() {
    let (pool, _guard, _tempdir, store) = setup_db("hail-cache-size-eviction").await;
    insert_message(&pool, store.as_ref(), "oldest", 3, 1, 60, false).await;
    insert_message(&pool, store.as_ref(), "middle", 2, 1, 60, false).await;
    let newest_blob = insert_message(&pool, store.as_ref(), "newest", 1, 1, 60, false).await;

    let stats = evict_account_bodies(
        &pool,
        store.as_ref(),
        1,
        &policy(MailCacheMode::Bounded, 30, 10, 100),
    )
    .await
    .expect("evict");

    assert_eq!(stats.evicted_bodies, 2);
    assert_eq!(body_blob(&pool, "oldest").await, None);
    assert_eq!(body_blob(&pool, "middle").await, None);
    assert_eq!(
        body_blob(&pool, "newest").await.as_deref(),
        Some(newest_blob.as_str())
    );
    assert_eq!(metadata_count(&pool).await, 3);
}

#[tokio::test]
async fn pinned_messages_are_never_evicted() {
    let (pool, _guard, _tempdir, store) = setup_db("hail-cache-pinned-eviction").await;
    let pinned_blob = insert_message(&pool, store.as_ref(), "pinned", 99, 99, 1_000, true).await;
    insert_message(&pool, store.as_ref(), "unpinned", 99, 99, 1_000, false).await;

    let stats = evict_account_bodies(
        &pool,
        store.as_ref(),
        1,
        &policy(MailCacheMode::Bounded, 1, 1, 1),
    )
    .await
    .expect("evict");

    assert_eq!(stats.evicted_bodies, 1);
    assert_eq!(
        body_blob(&pool, "pinned").await.as_deref(),
        Some(pinned_blob.as_str())
    );
    assert_eq!(body_blob(&pool, "unpinned").await, None);
}

#[tokio::test]
async fn screener_pending_messages_are_pinned_and_survive_eviction_until_decided() {
    let (pool, _guard, _tempdir, store) = setup_db("hail-cache-screener-pending-pin").await;
    let pending_blob = insert_message(&pool, store.as_ref(), "pending", 99, 99, 1_000, false).await;
    insert_message(&pool, store.as_ref(), "other", 99, 99, 1_000, false).await;
    sqlx::query(
        "UPDATE messages SET from_addr = 'pending@example.test' WHERE backend_msg_id = 'pending'",
    )
    .execute(&pool)
    .await
    .expect("set pending sender");
    sqlx::query(
        "INSERT INTO screener_rules (user_id, sender_address, decision, first_seen_at) \
         VALUES (1, 'pending@example.test', 'pending', '2026-01-01T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .expect("insert pending rule");

    assert_eq!(
        refresh_pinned_messages(&pool).await.expect("refresh pins"),
        1
    );
    let pinned: i64 =
        sqlx::query_scalar("SELECT pinned FROM messages WHERE backend_msg_id = 'pending'")
            .fetch_one(&pool)
            .await
            .expect("pending pinned");
    assert_eq!(pinned, 1);

    let stats = evict_account_bodies(
        &pool,
        store.as_ref(),
        1,
        &policy(MailCacheMode::Bounded, 1, 1, 1),
    )
    .await
    .expect("evict");

    assert_eq!(stats.evicted_bodies, 1);
    assert_eq!(
        body_blob(&pool, "pending").await.as_deref(),
        Some(pending_blob.as_str())
    );
    assert_eq!(body_blob(&pool, "other").await, None);

    sqlx::query(
        "UPDATE screener_rules SET decision = 'allow', classify_as = 'imbox', decided_at = '2026-01-02T00:00:00Z' \
         WHERE user_id = 1 AND sender_address = 'pending@example.test'",
    )
    .execute(&pool)
    .await
    .expect("approve sender");
    assert_eq!(
        refresh_pinned_messages(&pool).await.expect("refresh pins"),
        1
    );
    let pinned: i64 =
        sqlx::query_scalar("SELECT pinned FROM messages WHERE backend_msg_id = 'pending'")
            .fetch_one(&pool)
            .await
            .expect("pending unpinned");
    assert_eq!(pinned, 0);
}

#[tokio::test]
async fn full_and_off_modes_are_noops() {
    let (pool, _guard, _tempdir, store) = setup_db("hail-cache-mode-noop-eviction").await;
    let blob = insert_message(&pool, store.as_ref(), "old", 99, 99, 1_000, false).await;

    let off = evict_account_bodies(
        &pool,
        store.as_ref(),
        1,
        &policy(MailCacheMode::Off, 1, 1, 1),
    )
    .await
    .expect("off evict");
    let full = evict_account_bodies(
        &pool,
        store.as_ref(),
        1,
        &policy(MailCacheMode::Full, 1, 1, 1),
    )
    .await
    .expect("full evict");

    assert_eq!(off.evicted_bodies, 0);
    assert_eq!(full.evicted_bodies, 0);
    assert_eq!(
        body_blob(&pool, "old").await.as_deref(),
        Some(blob.as_str())
    );
}
