use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bytes::Bytes;
use futures_util::stream;
use hail_backend::{
    BackendMsgId, BlobRef, Capabilities, Change, Envelope, Keyword, MailBackend, Mailbox,
    MailboxRole, Page, PageRequest, Principal, Query, RawMessage, SubmissionId, SyncCursor,
};
use hail_blob_store::{BlobStore, FilesystemBlobStore};
use hail_cache::{CachePolicy, CachedMail, MailTarget};
use hail_core::{MailBackfill, MailCacheMode, MailClassification};
use sqlx::{Row, SqlitePool};
use tempfile::TempDir;

static CAPABILITIES: Capabilities = Capabilities {
    supports_initial_import: false,
    supports_eventsource: false,
    supports_principals_admin: false,
    supports_send: true,
    native_threading: false,
    max_attachment_size: 0,
    label_path_separator: '/',
};

#[derive(Clone)]
struct FakeBackend {
    messages: Arc<HashMap<BackendMsgId, RawMessage>>,
    order: Arc<Vec<BackendMsgId>>,
    mutation_calls: Arc<Mutex<usize>>,
}

impl FakeBackend {
    fn new(messages: Vec<RawMessage>) -> Self {
        let order = messages
            .iter()
            .map(|message| message.id.clone())
            .collect::<Vec<_>>();
        let messages = messages
            .into_iter()
            .map(|message| (message.id.clone(), message))
            .collect::<HashMap<_, _>>();
        Self {
            messages: Arc::new(messages),
            order: Arc::new(order),
            mutation_calls: Arc::new(Mutex::new(0)),
        }
    }

    fn mutation_calls(&self) -> usize {
        *self.mutation_calls.lock().expect("mutation lock")
    }
}

#[async_trait]
impl MailBackend for FakeBackend {
    fn capabilities(&self) -> &'static Capabilities {
        &CAPABILITIES
    }

    async fn list_message_ids(
        &self,
        _query: &Query,
        page: &PageRequest,
    ) -> hail_backend::Result<Page<BackendMsgId>> {
        let limit = usize::try_from(page.limit).unwrap_or(usize::MAX);
        Ok(Page {
            items: self.order.iter().take(limit).cloned().collect(),
            next_cursor: None,
        })
    }

    async fn get_message(&self, id: &BackendMsgId) -> hail_backend::Result<RawMessage> {
        self.messages
            .get(id)
            .cloned()
            .ok_or_else(|| hail_backend::Error::NotFound {
                kind: "message",
                id: id.as_str().to_owned(),
            })
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
        *self.mutation_calls.lock().expect("mutation lock") += 1;
        Ok(())
    }

    async fn move_to_role(
        &self,
        _id: &BackendMsgId,
        _role: MailboxRole,
    ) -> hail_backend::Result<()> {
        *self.mutation_calls.lock().expect("mutation lock") += 1;
        Ok(())
    }

    async fn delete_permanently(&self, _id: &BackendMsgId) -> hail_backend::Result<()> {
        *self.mutation_calls.lock().expect("mutation lock") += 1;
        Ok(())
    }

    async fn send(
        &self,
        _rfc822: &[u8],
        _envelope: &Envelope,
    ) -> hail_backend::Result<SubmissionId> {
        *self.mutation_calls.lock().expect("mutation lock") += 1;
        Ok(SubmissionId::new("fake-submission"))
    }

    async fn poll_changes(
        &self,
        cursor: &SyncCursor,
    ) -> hail_backend::Result<(Vec<Change>, SyncCursor)> {
        Ok((Vec::new(), cursor.clone()))
    }

    async fn watch_changes(&self) -> futures_util::stream::BoxStream<'static, Change> {
        Box::pin(stream::empty())
    }

    async fn list_mailboxes(&self) -> hail_backend::Result<Vec<Mailbox>> {
        Ok(Vec::new())
    }

    async fn list_principals(&self) -> hail_backend::Result<Vec<Principal>> {
        Ok(Vec::new())
    }
}

async fn fixture(
    messages: Vec<RawMessage>,
    mode: MailCacheMode,
) -> (CachedMail, FakeBackend, TempDir) {
    let pool = hail_db::connect("sqlite::memory:")
        .await
        .expect("connect in-memory sqlite");
    hail_db::migrate(&pool).await.expect("run migrations");
    ensure_default_account(&pool).await;

    let tempdir = tempfile::tempdir().expect("create temp blob dir");
    let blobs = Arc::new(FilesystemBlobStore::new(tempdir.path())) as Arc<dyn BlobStore>;
    let backend = FakeBackend::new(messages);
    let policy = CachePolicy::new(mode, 90, 50_000, 5 * 1024 * 1024, MailBackfill::Off);
    let cache = CachedMail::new(pool, blobs, Box::new(backend.clone()), policy);
    (cache, backend, tempdir)
}

async fn ensure_default_account(pool: &SqlitePool) {
    insert_account(pool, 1, "cache@example.test", "provider-acct").await;
}

async fn insert_account(pool: &SqlitePool, id: i64, email: &str, provider_account_id: &str) {
    sqlx::query(
        "INSERT INTO users (id, email, jmap_account_id, display_name, is_admin, created_at) \
         VALUES (?1, ?2, ?3, NULL, 1, '2026-01-01T00:00:00Z')",
    )
    .bind(id)
    .bind(email)
    .bind(format!("acct-{id}"))
    .execute(pool)
    .await
    .expect("insert user");
    sqlx::query(
        "INSERT INTO mail_accounts \
         (id, user_id, jmap_account_id, backend_kind, provider_kind, provider_account_id, provider_email, refresh_token_enc, sync_status, created_at, updated_at) \
         VALUES (?1, ?1, ?2, 'gmail', 'gmail', ?3, ?4, ?5, 'active', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .bind(id)
    .bind(format!("acct-{id}"))
    .bind(provider_account_id)
    .bind(email)
    .bind(vec![7_u8; 32])
    .execute(pool)
    .await
    .expect("insert account");
}

fn raw_message(id: &str, thread_id: &str, keywords: Vec<&str>) -> RawMessage {
    let mut metadata = BTreeMap::new();
    metadata.insert("subject".to_owned(), id.to_owned());
    metadata.insert("preview".to_owned(), format!("preview {id}"));
    RawMessage {
        id: BackendMsgId::new(id),
        thread_id: Some(thread_id.to_owned()),
        rfc822: Bytes::from_static(b"Subject: ignored\r\n\r\nbody intentionally not cached"),
        keywords: keywords.into_iter().map(Keyword::new).collect(),
        envelope: Some(Envelope {
            mail_from: format!("{id}@example.test"),
            rcpt_to: vec!["me@example.test".to_owned()],
        }),
        received_at_epoch_secs: Some(1_700_000_000),
        size_bytes: Some(1234),
        blob_refs: Vec::new(),
        attachments: Vec::new(),
        metadata,
    }
}

async fn cache_message(cache: &CachedMail, id: &str) {
    cache
        .get_message(&BackendMsgId::new(id))
        .await
        .expect("populate message metadata");
}

async fn message_keywords(cache: &CachedMail, id: &str) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT mk.keyword \
         FROM message_keywords mk \
         INNER JOIN messages m ON m.id = mk.message_id \
         WHERE m.backend_msg_id = ?1 \
         ORDER BY mk.keyword",
    )
    .bind(id)
    .fetch_all(cache.db())
    .await
    .expect("read message keywords")
}

async fn outbound_rows(cache: &CachedMail) -> Vec<(String, String, String)> {
    let rows = sqlx::query(
        "SELECT backend_msg_id, change_type, payload_json FROM outbound_changes ORDER BY id",
    )
    .fetch_all(cache.db())
    .await
    .expect("read outbound rows");
    rows.into_iter()
        .map(|row| {
            (
                row.get("backend_msg_id"),
                row.get("change_type"),
                row.get("payload_json"),
            )
        })
        .collect()
}

#[tokio::test]
async fn writethrough_mutate_keywords_updates_local_rows_and_inserts_outbound_rows() {
    let message = raw_message(
        "msg-1",
        "thread-1",
        vec![MailClassification::Imbox.keyword(), "$seen", "$custom_old"],
    );
    let (cache, backend, _tempdir) = fixture(vec![message], MailCacheMode::Bounded).await;
    cache_message(&cache, "msg-1").await;

    cache
        .mutate_keywords(
            MailTarget::Message(&BackendMsgId::new("msg-1")),
            &[Keyword::new("$custom_new")],
            &[Keyword::new("$seen"), Keyword::new("$custom_old")],
        )
        .await
        .expect("write-through keyword mutation");

    assert_eq!(
        backend.mutation_calls(),
        0,
        "write-through must not call backend"
    );
    assert_eq!(
        message_keywords(&cache, "msg-1").await,
        vec!["$custom_new", MailClassification::Imbox.keyword()]
    );
    assert_eq!(cache.pending_sync_count().await.expect("pending count"), 3);

    let rows = outbound_rows(&cache).await;
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].0, "msg-1");
    assert_eq!(rows[0].1, "unread");
    assert_eq!(rows[0].2, r#"{"keyword":"$seen"}"#);
    assert_eq!(rows[1].1, "keyword_remove");
    assert_eq!(rows[1].2, r#"{"keyword":"$custom_old"}"#);
    assert_eq!(rows[2].1, "keyword_add");
    assert_eq!(rows[2].2, r#"{"keyword":"$custom_new"}"#);
}

#[tokio::test]
async fn writethrough_pending_count_reflects_only_unapplied_rows_for_account() {
    let message = raw_message(
        "msg-1",
        "thread-1",
        vec![MailClassification::Imbox.keyword()],
    );
    let (cache, _backend, _tempdir) = fixture(vec![message], MailCacheMode::Bounded).await;

    cache
        .mutate_keywords(
            MailTarget::Message(&BackendMsgId::new("msg-1")),
            &[Keyword::new("$seen")],
            &[],
        )
        .await
        .expect("enqueue one row");
    cache
        .move_to_role(
            MailTarget::Message(&BackendMsgId::new("msg-1")),
            MailboxRole::Trash,
        )
        .await
        .expect("enqueue second row");

    sqlx::query("UPDATE outbound_changes SET applied_at = ?1 WHERE change_type = 'read'")
        .bind("2026-01-01T00:00:00Z")
        .execute(cache.db())
        .await
        .expect("mark one applied");
    insert_account(cache.db(), 2, "other@example.test", "provider-other").await;
    sqlx::query(
        "INSERT INTO outbound_changes (account_id, backend_msg_id, change_type, payload_json, created_at) \
         VALUES (2, 'other', 'read', '{}', '2026-01-01T00:00:00Z')",
    )
    .execute(cache.db())
    .await
    .expect("insert other account pending row");

    assert_eq!(cache.pending_sync_count().await.expect("pending count"), 1);
}

#[tokio::test]
async fn writethrough_move_to_role_updates_cached_thread_and_enqueues_per_message_rows() {
    let first = raw_message(
        "msg-1",
        "thread-1",
        vec![MailClassification::Imbox.keyword()],
    );
    let second = raw_message("msg-2", "thread-1", vec!["$archive"]);
    let (cache, _backend, _tempdir) = fixture(vec![first, second], MailCacheMode::Bounded).await;
    cache_message(&cache, "msg-1").await;
    cache_message(&cache, "msg-2").await;

    cache
        .move_to_role(MailTarget::Thread("thread-1"), MailboxRole::Trash)
        .await
        .expect("move thread to trash");

    assert_eq!(message_keywords(&cache, "msg-1").await, vec!["$trash"]);
    assert_eq!(message_keywords(&cache, "msg-2").await, vec!["$trash"]);
    assert_eq!(cache.pending_sync_count().await.expect("pending count"), 2);

    let rows = outbound_rows(&cache).await;
    assert_eq!(
        rows[0],
        (
            "msg-1".to_owned(),
            "trash".to_owned(),
            r#"{"role":"trash"}"#.to_owned()
        )
    );
    assert_eq!(
        rows[1],
        (
            "msg-2".to_owned(),
            "trash".to_owned(),
            r#"{"role":"trash"}"#.to_owned()
        )
    );
}

#[tokio::test]
async fn writethrough_cache_off_enqueues_without_message_rows() {
    let (cache, backend, _tempdir) = fixture(Vec::new(), MailCacheMode::Off).await;

    cache
        .mutate_keywords(
            MailTarget::Message(&BackendMsgId::new("offline-msg")),
            &[Keyword::new("$seen")],
            &[],
        )
        .await
        .expect("enqueue while cache off");

    assert_eq!(backend.mutation_calls(), 0, "mutation should only queue");
    assert_eq!(cache.pending_sync_count().await.expect("pending count"), 1);
    let message_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
        .fetch_one(cache.db())
        .await
        .expect("count messages");
    assert_eq!(message_rows, 0);
    assert_eq!(
        outbound_rows(&cache).await,
        vec![(
            "offline-msg".to_owned(),
            "read".to_owned(),
            r#"{"keyword":"$seen"}"#.to_owned(),
        )]
    );
}

#[tokio::test]
async fn sync_message_created_upserts_metadata_without_body_blob() {
    let message = raw_message(
        "msg-created",
        "thread-created",
        vec![MailClassification::Imbox.keyword(), "$seen"],
    );
    let (cache, backend, _tempdir) = fixture(vec![message.clone()], MailCacheMode::Bounded).await;

    cache
        .apply_change(Change::MessageCreated {
            id: BackendMsgId::new("msg-created"),
            raw_ref: Some(message),
        })
        .await
        .expect("apply created");

    assert_eq!(backend.mutation_calls(), 0);
    let row: (String, Option<String>) = sqlx::query_as(
        "SELECT subject, body_blob_id FROM messages WHERE backend_msg_id = 'msg-created'",
    )
    .fetch_one(cache.db())
    .await
    .expect("message row");
    assert_eq!(row.0, "msg-created");
    assert_eq!(row.1, None);
    assert_eq!(
        message_keywords(&cache, "msg-created").await,
        vec!["$hail_imbox", "$seen"]
    );
}

#[tokio::test]
async fn sync_message_updated_mutates_keywords() {
    let message = raw_message(
        "msg-update",
        "thread-update",
        vec![MailClassification::Imbox.keyword(), "$seen", "$old"],
    );
    let (cache, _backend, _tempdir) = fixture(vec![message], MailCacheMode::Bounded).await;
    cache_message(&cache, "msg-update").await;

    cache
        .apply_change(Change::MessageUpdated {
            id: BackendMsgId::new("msg-update"),
            keywords: None,
            keywords_added: vec![Keyword::new("$new")],
            keywords_removed: vec![Keyword::new("$seen"), Keyword::new("$old")],
        })
        .await
        .expect("apply update");

    assert_eq!(
        message_keywords(&cache, "msg-update").await,
        vec!["$hail_imbox", "$new"]
    );
}

#[tokio::test]
async fn sync_message_deleted_removes_cached_row() {
    let message = raw_message(
        "msg-delete",
        "thread-delete",
        vec![MailClassification::Imbox.keyword()],
    );
    let (cache, _backend, _tempdir) = fixture(vec![message], MailCacheMode::Bounded).await;
    cache_message(&cache, "msg-delete").await;

    cache
        .apply_change(Change::MessageDeleted {
            id: BackendMsgId::new("msg-delete"),
        })
        .await
        .expect("apply delete");

    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE backend_msg_id = 'msg-delete'")
            .fetch_one(cache.db())
            .await
            .expect("count messages");
    assert_eq!(count, 0);
}

#[tokio::test]
async fn sync_lww_keeps_newer_pending_local_change() {
    let message = raw_message(
        "msg-lww",
        "thread-lww",
        vec![MailClassification::Imbox.keyword()],
    );
    let (cache, _backend, _tempdir) = fixture(vec![message], MailCacheMode::Bounded).await;
    cache_message(&cache, "msg-lww").await;
    sqlx::query(
        r#"INSERT INTO outbound_changes (account_id, backend_msg_id, change_type, payload_json, created_at)
           VALUES (1, 'msg-lww', 'read', '{"keyword":"$seen"}', ?1)"#,
    )
    .bind((chrono::Utc::now() + chrono::Duration::seconds(10)).to_rfc3339())
    .execute(cache.db())
    .await
    .expect("insert newer pending row");

    cache
        .apply_change(Change::MessageUpdated {
            id: BackendMsgId::new("msg-lww"),
            keywords: None,
            keywords_added: vec![Keyword::new("$seen")],
            keywords_removed: Vec::new(),
        })
        .await
        .expect("apply older incoming update");

    assert_eq!(
        message_keywords(&cache, "msg-lww").await,
        vec!["$hail_imbox"]
    );
}

#[tokio::test]
async fn sync_ignores_recent_self_applied_change_within_sixty_seconds() {
    let message = raw_message(
        "msg-loop",
        "thread-loop",
        vec![MailClassification::Imbox.keyword()],
    );
    let (cache, _backend, _tempdir) = fixture(vec![message], MailCacheMode::Bounded).await;
    cache_message(&cache, "msg-loop").await;
    sqlx::query(
        r#"INSERT INTO outbound_changes (account_id, backend_msg_id, change_type, payload_json, created_at, applied_at)
           VALUES (1, 'msg-loop', 'read', '{"keyword":"$seen"}', ?1, ?1)"#,
    )
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(cache.db())
    .await
    .expect("insert applied row");

    cache
        .apply_change(Change::MessageUpdated {
            id: BackendMsgId::new("msg-loop"),
            keywords: None,
            keywords_added: vec![Keyword::new("$seen")],
            keywords_removed: Vec::new(),
        })
        .await
        .expect("apply self echo");

    assert_eq!(
        message_keywords(&cache, "msg-loop").await,
        vec!["$hail_imbox"]
    );
}

#[tokio::test]
async fn sync_trash_is_not_permanently_deleted_or_untrashed() {
    let message = raw_message("msg-trash", "thread-trash", vec!["$trash"]);
    let (cache, _backend, _tempdir) = fixture(vec![message], MailCacheMode::Bounded).await;
    cache_message(&cache, "msg-trash").await;

    cache
        .apply_changes([
            Change::MailboxRoleChanged {
                id: BackendMsgId::new("msg-trash"),
                role: MailboxRole::Inbox,
            },
            Change::MessageDeleted {
                id: BackendMsgId::new("msg-trash"),
            },
        ])
        .await
        .expect("apply trash-conflicting changes");

    assert_eq!(message_keywords(&cache, "msg-trash").await, vec!["$trash"]);
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE backend_msg_id = 'msg-trash'")
            .fetch_one(cache.db())
            .await
            .expect("count messages");
    assert_eq!(count, 1);
}
