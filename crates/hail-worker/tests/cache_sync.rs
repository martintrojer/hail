use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use bytes::Bytes;
use futures_util::stream;
use hail_backend::{
    BackendMsgId, BlobRef, Capabilities, Change, Envelope, Keyword, MailBackend, Mailbox,
    MailboxRole, Page, PageRequest, Principal, Query, RawMessage, SubmissionId, SyncCursor,
};
use hail_blob_store::FilesystemBlobStore;
use hail_cache::CachePolicy;
use hail_core::{MailBackfill, MailCacheMode};
use hail_test::{TempDb, fresh_db_url};
use hail_worker::cache_sync::{
    CacheSyncOptions, SyncAccount, run_cache_sync_once, run_cache_sync_poll_loop,
};
use sqlx::SqlitePool;
use tokio_util::sync::CancellationToken;

const CAPS: Capabilities = Capabilities {
    supports_initial_import: false,
    supports_eventsource: true,
    supports_principals_admin: false,
    supports_send: true,
    native_threading: true,
    max_attachment_size: u64::MAX,
    label_path_separator: '/',
};

#[derive(Clone)]
struct FakeBackend {
    changes: Arc<Vec<Change>>,
    polls: Arc<AtomicUsize>,
}

#[async_trait]
impl MailBackend for FakeBackend {
    fn capabilities(&self) -> &'static Capabilities {
        &CAPS
    }

    async fn list_message_ids(
        &self,
        _query: &Query,
        _page: &PageRequest,
    ) -> hail_backend::Result<Page<BackendMsgId>> {
        Ok(Page::empty())
    }

    async fn get_message(&self, id: &BackendMsgId) -> hail_backend::Result<RawMessage> {
        Ok(raw_message(id.as_str()))
    }

    async fn fetch_blob(&self, _id: &BlobRef) -> hail_backend::Result<Bytes> {
        Ok(Bytes::new())
    }

    async fn set_keywords(
        &self,
        _id: &BackendMsgId,
        _add: &[Keyword],
        _remove: &[Keyword],
    ) -> hail_backend::Result<()> {
        Ok(())
    }

    async fn move_to_role(
        &self,
        _id: &BackendMsgId,
        _role: MailboxRole,
    ) -> hail_backend::Result<()> {
        Ok(())
    }

    async fn delete_permanently(&self, _id: &BackendMsgId) -> hail_backend::Result<()> {
        Ok(())
    }

    async fn send(
        &self,
        _rfc822: &[u8],
        _envelope: &Envelope,
    ) -> hail_backend::Result<SubmissionId> {
        Ok(SubmissionId::new("sent"))
    }

    async fn poll_changes(
        &self,
        cursor: &SyncCursor,
    ) -> hail_backend::Result<(Vec<Change>, SyncCursor)> {
        self.polls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(cursor.as_str(), "cursor-before");
        Ok((
            self.changes.as_ref().clone(),
            SyncCursor::new("cursor-after"),
        ))
    }

    async fn watch_changes(&self) -> futures_core::stream::BoxStream<'static, Change> {
        Box::pin(stream::iter(self.changes.as_ref().clone()))
    }

    async fn list_mailboxes(&self) -> hail_backend::Result<Vec<Mailbox>> {
        Ok(Vec::new())
    }

    async fn list_principals(&self) -> hail_backend::Result<Vec<Principal>> {
        Ok(Vec::new())
    }
}

fn raw_message(id: &str) -> RawMessage {
    RawMessage {
        id: BackendMsgId::new(id),
        thread_id: Some(format!("thread-{id}")),
        rfc822: Bytes::from(format!(
            "From: sender@example.test\r\nSubject: {id}\r\n\r\nbody"
        )),
        keywords: vec![Keyword::new("$seen")],
        envelope: None,
        received_at_epoch_secs: Some(1_700_000_000),
        size_bytes: Some(42),
        blob_refs: Vec::new(),
        attachments: Vec::new(),
        metadata: Default::default(),
    }
}

async fn setup_db(name: &str) -> (SqlitePool, TempDb, tempfile::TempDir) {
    let (url, guard) = fresh_db_url(name);
    let pool = hail_db::connect(&url).await.expect("connect");
    hail_db::migrate(&pool).await.expect("migrate");
    sqlx::query(
        "INSERT INTO users (id, email, jmap_account_id, display_name, is_admin, created_at) \
         VALUES (1, 'sync@example.test', 'acct', NULL, 1, '2026-01-01T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .expect("insert user");
    sqlx::query(
        "INSERT INTO mail_accounts \
         (id, user_id, jmap_account_id, backend_kind, provider_kind, provider_account_id, provider_email, refresh_token_enc, sync_status, last_profile_history_id, created_at, updated_at) \
         VALUES (1, 1, 'acct', 'gmail', 'gmail', 'provider-acct', 'sync@example.test', ?1, 'active', 'cursor-before', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .bind(vec![7_u8; 32])
    .execute(&pool)
    .await
    .expect("insert account");
    (pool, guard, tempfile::tempdir().expect("tempdir"))
}

#[tokio::test]
async fn sync_once_polls_backend_applies_change_and_advances_cursor() {
    let (pool, _guard, tempdir) = setup_db("hail-worker-cache-sync-once").await;
    let polls = Arc::new(AtomicUsize::new(0));
    let backend = FakeBackend {
        changes: Arc::new(vec![Change::MessageCreated {
            id: BackendMsgId::new("msg-1"),
            raw_ref: Some(raw_message("msg-1")),
        }]),
        polls: Arc::clone(&polls),
    };
    let account = SyncAccount {
        account_id: 1,
        user_id: 1,
        cursor: Some("cursor-before".to_owned()),
        policy: CachePolicy::new(
            MailCacheMode::Bounded,
            90,
            50_000,
            1024 * 1024,
            MailBackfill::Off,
        ),
    };

    let summary = run_cache_sync_once(
        &pool,
        Arc::new(FilesystemBlobStore::new(tempdir.path())),
        vec![account],
        move |_| {
            let backend = backend.clone();
            async move { Some(Box::new(backend) as Box<dyn MailBackend + Send + Sync>) }
        },
        &CancellationToken::new(),
    )
    .await
    .expect("sync once");

    assert_eq!(summary.changes_applied, 1);
    assert_eq!(polls.load(Ordering::SeqCst), 1);
    let subject: String =
        sqlx::query_scalar("SELECT subject FROM messages WHERE backend_msg_id = 'msg-1'")
            .fetch_one(&pool)
            .await
            .expect("cached message");
    assert_eq!(subject, "");
    let cursor: String =
        sqlx::query_scalar("SELECT last_profile_history_id FROM mail_accounts WHERE id = 1")
            .fetch_one(&pool)
            .await
            .expect("cursor");
    assert_eq!(cursor, "cursor-after");
}

#[tokio::test]
async fn sync_poll_loop_cancels_promptly() {
    let (pool, _guard, tempdir) = setup_db("hail-worker-cache-sync-cancel").await;
    let cancel = CancellationToken::new();
    cancel.cancel();

    let result = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        run_cache_sync_poll_loop(
            pool,
            Arc::new(FilesystemBlobStore::new(tempdir.path())),
            |_| async { None },
            CacheSyncOptions {
                poll_interval: std::time::Duration::from_secs(60),
            },
            cancel,
        ),
    )
    .await;

    assert!(
        result.is_ok(),
        "sync loop should observe cancellation promptly"
    );
    result.expect("timeout").expect("sync loop result");
}
