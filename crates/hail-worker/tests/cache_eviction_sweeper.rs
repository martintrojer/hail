use std::time::Duration;

use hail_blob_store::{BlobStore, FilesystemBlobStore};
use hail_core::{BlobKind, MailBackfill, MailCacheMode};
use hail_test::{TempDb, fresh_db_url};
use hail_worker::cache_eviction_sweeper::{
    CacheEvictionSweeperOptions, run_cache_eviction_sweep_once, run_cache_eviction_sweeper,
};
use sqlx::SqlitePool;
use tokio_util::sync::CancellationToken;

async fn setup_db(name: &str) -> (SqlitePool, TempDb, tempfile::TempDir, FilesystemBlobStore) {
    let (url, guard) = fresh_db_url(name);
    let pool = hail_db::connect(&url).await.expect("connect");
    hail_db::migrate(&pool).await.expect("migrate");
    ensure_account(&pool).await;
    let tempdir = tempfile::tempdir().expect("blob tempdir");
    let store = FilesystemBlobStore::new(tempdir.path()).with_sweep_grace(Duration::ZERO);
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

async fn insert_bounded_policy(pool: &SqlitePool) {
    sqlx::query(
        "INSERT INTO cache_policy (account_id, mode, keep_days, keep_max_msgs, keep_max_bytes, backfill, updated_at) \
         VALUES (1, ?1, 1, 1, 1, ?2, '2026-01-01T00:00:00Z')",
    )
    .bind(match MailCacheMode::Bounded {
        MailCacheMode::Off => "off",
        MailCacheMode::Bounded => "bounded",
        MailCacheMode::Full => "full",
    })
    .bind(match MailBackfill::Off {
        MailBackfill::Off => "off",
        MailBackfill::Incremental => "incremental",
    })
    .execute(pool)
    .await
    .expect("insert policy");
}

async fn insert_message(pool: &SqlitePool, store: &dyn BlobStore, backend_id: &str) {
    let blob_id = store
        .put(BlobKind::Eml, format!("body {backend_id}").as_bytes())
        .await
        .expect("put blob")
        .to_string();
    sqlx::query(
        "INSERT INTO messages \
         (account_id, backend_msg_id, thread_id, internal_date, from_addr, subject, preview, size_bytes, body_blob_id, body_text, inserted_at, accessed_at, pinned) \
         VALUES (1, ?1, ?2, 1, 'sender@example.test', ?1, 'preview', 10, ?3, 'body text', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 0)",
    )
    .bind(backend_id)
    .bind(format!("thread-{backend_id}"))
    .bind(blob_id)
    .execute(pool)
    .await
    .expect("insert message");
}

#[tokio::test]
async fn sweep_once_iterates_policies_and_evicts() {
    let (pool, _guard, _tempdir, store) = setup_db("hail-worker-cache-evict-once").await;
    insert_bounded_policy(&pool).await;
    insert_message(&pool, &store, "old").await;

    let summary = run_cache_eviction_sweep_once(&pool, &store)
        .await
        .expect("sweep once");

    assert_eq!(summary.accounts_considered, 1);
    assert_eq!(summary.evicted_bodies, 1);
    let body_blob: Option<String> =
        sqlx::query_scalar("SELECT body_blob_id FROM messages WHERE backend_msg_id = 'old'")
            .fetch_one(&pool)
            .await
            .expect("body blob");
    assert_eq!(body_blob, None);
}

#[tokio::test]
async fn sweeper_loop_cancels_promptly() {
    let (pool, _guard, _tempdir, store) = setup_db("hail-worker-cache-evict-cancel").await;
    let cancel = CancellationToken::new();
    cancel.cancel();

    let result = tokio::time::timeout(
        Duration::from_millis(100),
        run_cache_eviction_sweeper(
            pool,
            store,
            CacheEvictionSweeperOptions {
                interval: Duration::from_secs(60),
            },
            cancel,
        ),
    )
    .await;

    assert!(
        result.is_ok(),
        "sweeper should observe cancellation promptly"
    );
    result.expect("timeout").expect("sweeper result");
}
