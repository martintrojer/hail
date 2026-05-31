use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use hail_db::provider_message_mappings::{ProviderImportStatus, get_provider_message_mapping};
use hail_db::provider_sync_audit::list_provider_sync_audit_logs;
use hail_test::gmail_import_fixtures::{
    GmailImportFixture, GmailImportScenario, gmail_import_fixture,
};
use hail_worker::gmail_client::{
    GmailApiErrorKind, GmailClientError, GmailHistoryMessage, GmailHistoryMessageRef,
    GmailHistoryRecord, GmailLabel, ListHistoryParams, ListHistoryResponse, ListMessage,
    ListMessagesParams, ListMessagesResponse, RawGmailMessage,
};
use hail_worker::gmail_historical_import::{GmailHistoricalImporter, GmailHistoricalSource};
use hail_worker::gmail_incremental_sync::{
    GmailIncrementalSource, GmailIncrementalSyncAccount, GmailIncrementalSyncError,
    GmailIncrementalSyncOptions, run_gmail_incremental_sync,
};
use hail_worker::provider_import_routing::RoutedImportedRfc822Message;
use hail_worker::rfc822_import::{
    FakeRfc822Importer, Rfc822ImportError, Rfc822ImportRequest, Rfc822Importer,
};
use reqwest::StatusCode;
use tokio::sync::Barrier;
use tokio_util::sync::CancellationToken;

fn fresh_db_url() -> (String, TempDb) {
    let mut dir = std::env::temp_dir();
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time")
        .as_nanos();

    for attempt in 0..100_u8 {
        dir.push(format!(
            "hail-worker-gmail-incremental-sync-test-{pid}-{nanos}-{attempt}"
        ));
        match std::fs::create_dir(&dir) {
            Ok(()) => {
                let path = dir.join("hail.db");
                let url = format!("sqlite://{}", path.display());
                return (url, TempDb { dir, path });
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => dir.pop(),
            Err(err) => panic!("create temp db dir: {err}"),
        };
    }
    panic!("failed to allocate unique temp db dir");
}

struct TempDb {
    dir: std::path::PathBuf,
    path: std::path::PathBuf,
}
impl Drop for TempDb {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_file(self.path.with_extension("db-wal"));
        let _ = std::fs::remove_file(self.path.with_extension("db-shm"));
        let _ = std::fs::remove_dir(&self.dir);
    }
}

async fn setup(history_id: Option<&str>) -> (sqlx::SqlitePool, TempDb, i64, i64) {
    let (url, guard) = fresh_db_url();
    let pool = hail_db::connect(&url).await.expect("connect");
    hail_db::migrate(&pool).await.expect("migrate");
    let user_id = insert_user(&pool, "sync@example.com", "acct-sync").await;
    let provider_account_id = insert_provider_account(&pool, user_id, history_id).await;
    (pool, guard, user_id, provider_account_id)
}

async fn insert_user(pool: &sqlx::SqlitePool, email: &str, account_id: &str) -> i64 {
    sqlx::query("INSERT INTO users (email, jmap_account_id, created_at) VALUES (?, ?, ?)")
        .bind(email)
        .bind(account_id)
        .bind("2026-01-01T00:00:00Z")
        .execute(pool)
        .await
        .expect("user insert");
    sqlx::query_scalar("SELECT id FROM users WHERE email = ?")
        .bind(email)
        .fetch_one(pool)
        .await
        .expect("user id")
}

async fn insert_provider_account(
    pool: &sqlx::SqlitePool,
    user_id: i64,
    history_id: Option<&str>,
) -> i64 {
    sqlx::query(
        "INSERT INTO provider_accounts \
         (user_id, jmap_account_id, provider_kind, provider_account_id, provider_email, \
          refresh_token_enc, last_profile_history_id, sync_status, created_at, updated_at) \
         VALUES (?, 'acct-sync', 'gmail', ?, ?, ?, ?, 'active', ?, ?)",
    )
    .bind(user_id)
    .bind(format!("gmail-provider-{user_id}"))
    .bind(format!("user-{user_id}@gmail.example"))
    .bind(vec![1_u8; 29])
    .bind(history_id)
    .bind("2026-01-01T00:00:00Z")
    .bind("2026-01-01T00:00:00Z")
    .execute(pool)
    .await
    .expect("provider account insert");

    sqlx::query_scalar("SELECT id FROM provider_accounts WHERE user_id = ?")
        .bind(user_id)
        .fetch_one(pool)
        .await
        .expect("provider account id")
}

#[derive(Clone, Debug)]
struct FakeGmail {
    history_pages: Arc<Mutex<Vec<Result<ListHistoryResponse, GmailClientError>>>>,
    message_pages: Arc<Mutex<Vec<ListMessagesResponse>>>,
    raw: Arc<HashMap<String, RawGmailMessage>>,
    labels: Arc<Vec<GmailLabel>>,
    history_params: Arc<Mutex<Vec<ListHistoryParams>>>,
    raw_gets: Arc<Mutex<Vec<String>>>,
}

impl FakeGmail {
    fn new(
        history_pages: Vec<Result<ListHistoryResponse, GmailClientError>>,
        message_pages: Vec<ListMessagesResponse>,
        raw: Vec<RawGmailMessage>,
    ) -> Self {
        Self::with_labels(history_pages, message_pages, raw, Vec::new())
    }

    fn with_labels(
        history_pages: Vec<Result<ListHistoryResponse, GmailClientError>>,
        message_pages: Vec<ListMessagesResponse>,
        raw: Vec<RawGmailMessage>,
        labels: Vec<GmailLabel>,
    ) -> Self {
        Self {
            history_pages: Arc::new(Mutex::new(history_pages)),
            message_pages: Arc::new(Mutex::new(message_pages)),
            raw: Arc::new(raw.into_iter().map(|msg| (msg.id.clone(), msg)).collect()),
            labels: Arc::new(labels),
            history_params: Arc::new(Mutex::new(Vec::new())),
            raw_gets: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn history_params(&self) -> Vec<ListHistoryParams> {
        self.history_params.lock().expect("history params").clone()
    }

    fn raw_gets(&self) -> Vec<String> {
        self.raw_gets.lock().expect("raw gets").clone()
    }
}

#[async_trait]
impl GmailIncrementalSource for FakeGmail {
    async fn list_history(
        &self,
        params: &ListHistoryParams,
    ) -> Result<ListHistoryResponse, GmailClientError> {
        self.history_params
            .lock()
            .expect("history params")
            .push(params.clone());
        self.history_pages.lock().expect("history pages").remove(0)
    }
}

#[async_trait]
impl GmailHistoricalSource for FakeGmail {
    async fn list_messages(
        &self,
        _params: &ListMessagesParams,
    ) -> Result<ListMessagesResponse, GmailClientError> {
        Ok(self.message_pages.lock().expect("message pages").remove(0))
    }

    async fn get_raw_message(&self, message_id: &str) -> Result<RawGmailMessage, GmailClientError> {
        self.raw_gets
            .lock()
            .expect("raw gets")
            .push(message_id.to_owned());
        self.raw
            .get(message_id)
            .cloned()
            .ok_or(GmailClientError::MissingRawMessage)
    }

    async fn list_user_labels(&self) -> Result<Vec<GmailLabel>, GmailClientError> {
        Ok(self.labels.as_ref().clone())
    }
}

#[derive(Debug)]
struct BlockingIncrementalGmail {
    history_entered: Barrier,
    raw_entered: Barrier,
    block_history: bool,
}

impl BlockingIncrementalGmail {
    fn history_blocked() -> Self {
        Self {
            history_entered: Barrier::new(2),
            raw_entered: Barrier::new(1),
            block_history: true,
        }
    }

    fn raw_blocked() -> Self {
        Self {
            history_entered: Barrier::new(1),
            raw_entered: Barrier::new(2),
            block_history: false,
        }
    }
}

#[async_trait]
impl GmailIncrementalSource for BlockingIncrementalGmail {
    async fn list_history(
        &self,
        _params: &ListHistoryParams,
    ) -> Result<ListHistoryResponse, GmailClientError> {
        self.history_entered.wait().await;
        if self.block_history {
            std::future::pending().await
        } else {
            Ok(history_page(
                vec![("101", vec![("gmail-block-raw", "thread-block")])],
                None,
                Some("102"),
            ))
        }
    }
}

#[async_trait]
impl GmailHistoricalSource for BlockingIncrementalGmail {
    async fn list_messages(
        &self,
        _params: &ListMessagesParams,
    ) -> Result<ListMessagesResponse, GmailClientError> {
        unreachable!("incremental cancellation tests should not run fallback list")
    }

    async fn get_raw_message(&self, message_id: &str) -> Result<RawGmailMessage, GmailClientError> {
        self.raw_entered.wait().await;
        Ok(match message_id {
            "gmail-block-raw" => std::future::pending().await,
            _ => raw_message(
                message_id,
                "thread-block",
                "history-block",
                "block@example.com",
            ),
        })
    }
}

fn history_page(
    records: Vec<(&str, Vec<(&str, &str)>)>,
    next_page_token: Option<&str>,
    history_id: Option<&str>,
) -> ListHistoryResponse {
    ListHistoryResponse {
        history: records
            .into_iter()
            .map(|(id, messages)| GmailHistoryRecord {
                id: id.to_owned(),
                messages_added: messages
                    .into_iter()
                    .map(|(id, thread_id)| GmailHistoryMessageRef {
                        message: GmailHistoryMessage {
                            id: id.to_owned(),
                            thread_id: Some(thread_id.to_owned()),
                        },
                    })
                    .collect(),
                messages: Vec::new(),
            })
            .collect(),
        next_page_token: next_page_token.map(str::to_owned),
        history_id: history_id.map(str::to_owned),
    }
}

fn raw_message(id: &str, thread_id: &str, history_id: &str, message_id: &str) -> RawGmailMessage {
    RawGmailMessage {
        id: id.to_owned(),
        thread_id: Some(thread_id.to_owned()),
        history_id: Some(history_id.to_owned()),
        label_ids: Vec::new(),
        rfc822: format!(
            "From: sender@example.com\r\nTo: user@example.com\r\nMessage-ID: <{message_id}>\r\nSubject: hi\r\n\r\nBody"
        )
        .into_bytes(),
    }
}

fn raw_fixture_message(fixture: GmailImportFixture) -> RawGmailMessage {
    RawGmailMessage {
        id: fixture.gmail_id.to_owned(),
        thread_id: Some(fixture.thread_id.to_owned()),
        history_id: Some(fixture.history_id.to_owned()),
        label_ids: Vec::new(),
        rfc822: fixture.raw_rfc822().to_vec(),
    }
}

fn gmail_label(id: &str, name: &str, label_type: &str) -> GmailLabel {
    GmailLabel {
        id: id.to_owned(),
        name: name.to_owned(),
        label_type: Some(label_type.to_owned()),
    }
}

fn raw_message_with_labels(
    id: &str,
    thread_id: &str,
    history_id: &str,
    message_id: &str,
    label_ids: &[&str],
) -> RawGmailMessage {
    let mut raw = raw_message(id, thread_id, history_id, message_id);
    raw.label_ids = label_ids.iter().map(|label| (*label).to_owned()).collect();
    raw
}

async fn thread_label_names(pool: &sqlx::SqlitePool, user_id: i64, thread_id: &str) -> Vec<String> {
    hail_db::labels::list_thread_labels(pool, user_id, thread_id)
        .await
        .expect("thread labels")
        .into_iter()
        .map(|label| label.name)
        .collect()
}

fn message_page_from_fixtures(
    fixtures: impl IntoIterator<Item = GmailImportFixture>,
) -> ListMessagesResponse {
    ListMessagesResponse {
        messages: fixtures
            .into_iter()
            .map(|fixture| ListMessage {
                id: fixture.gmail_id.to_owned(),
                thread_id: Some(fixture.thread_id.to_owned()),
            })
            .collect(),
        next_page_token: None,
        result_size_estimate: None,
    }
}

#[derive(Debug)]
struct PostImportCrashImporter<'a> {
    inner: &'a FakeRfc822Importer,
    fail_next_for_provider_id: Mutex<Option<String>>,
}

impl<'a> PostImportCrashImporter<'a> {
    fn new(inner: &'a FakeRfc822Importer, provider_message_id: impl Into<String>) -> Self {
        Self {
            inner,
            fail_next_for_provider_id: Mutex::new(Some(provider_message_id.into())),
        }
    }
}

#[async_trait]
impl GmailHistoricalImporter for PostImportCrashImporter<'_> {
    async fn import_gmail_rfc822(
        &self,
        _db: &sqlx::SqlitePool,
        _user_id: i64,
        request: Rfc822ImportRequest,
    ) -> Result<
        RoutedImportedRfc822Message,
        hail_worker::gmail_historical_import::GmailHistoricalImportError,
    > {
        let imported = self.inner.import_rfc822(request.clone()).await?;
        let mut fail_next = self
            .fail_next_for_provider_id
            .lock()
            .expect("post import crash flag");
        if fail_next.as_deref() == request.provider_message_id.as_deref() {
            *fail_next = None;
            return Err(
                hail_worker::gmail_historical_import::GmailHistoricalImportError::Rfc822Import(
                    Rfc822ImportError::Jmap(
                        "simulated crash after local incremental import".to_owned(),
                    ),
                ),
            );
        }
        Ok(RoutedImportedRfc822Message {
            imported,
            route_outcome: None,
        })
    }
}

async fn profile_history_cursor(
    pool: &sqlx::SqlitePool,
    provider_account_id: i64,
) -> Option<String> {
    sqlx::query_scalar("SELECT last_profile_history_id FROM provider_accounts WHERE id = ?1")
        .bind(provider_account_id)
        .fetch_one(pool)
        .await
        .expect("history cursor")
}

#[tokio::test]
async fn cancellation_interrupts_blocked_incremental_history_fetch() {
    let (pool, _guard, user_id, provider_account_id) = setup(Some("100")).await;
    let gmail = Arc::new(BlockingIncrementalGmail::history_blocked());
    let importer = Arc::new(FakeRfc822Importer::default());
    let cancel = CancellationToken::new();
    let task_pool = pool.clone();
    let task_gmail = gmail.clone();
    let task_importer = importer.clone();
    let task_cancel = cancel.clone();
    let handle = tokio::spawn(async move {
        run_gmail_incremental_sync(
            &task_pool,
            GmailIncrementalSyncAccount {
                provider_account_id,
                user_id,
                history_id: Some("100".to_owned()),
            },
            task_gmail.as_ref(),
            task_importer.as_ref(),
            GmailIncrementalSyncOptions::into_mailboxes(["inbox"]),
            &task_cancel,
        )
        .await
    });

    gmail.history_entered.wait().await;
    cancel.cancel();
    let err = tokio::time::timeout(std::time::Duration::from_millis(200), handle)
        .await
        .expect("incremental sync should stop promptly")
        .expect("join")
        .expect_err("cancelled sync");
    assert!(matches!(err, GmailIncrementalSyncError::Cancelled));
    assert_eq!(
        account_error_class(&pool, provider_account_id)
            .await
            .as_deref(),
        Some("cancelled")
    );
    assert!(importer.imports().is_empty());
}

#[tokio::test]
async fn cancellation_interrupts_blocked_incremental_raw_fetch() {
    let (pool, _guard, user_id, provider_account_id) = setup(Some("100")).await;
    let gmail = Arc::new(BlockingIncrementalGmail::raw_blocked());
    let importer = Arc::new(FakeRfc822Importer::default());
    let cancel = CancellationToken::new();
    let task_pool = pool.clone();
    let task_gmail = gmail.clone();
    let task_importer = importer.clone();
    let task_cancel = cancel.clone();
    let handle = tokio::spawn(async move {
        run_gmail_incremental_sync(
            &task_pool,
            GmailIncrementalSyncAccount {
                provider_account_id,
                user_id,
                history_id: Some("100".to_owned()),
            },
            task_gmail.as_ref(),
            task_importer.as_ref(),
            GmailIncrementalSyncOptions::into_mailboxes(["inbox"]),
            &task_cancel,
        )
        .await
    });

    gmail.raw_entered.wait().await;
    cancel.cancel();
    let err = tokio::time::timeout(std::time::Duration::from_millis(200), handle)
        .await
        .expect("incremental sync should stop promptly")
        .expect("join")
        .expect_err("cancelled sync");
    assert!(matches!(err, GmailIncrementalSyncError::Import(_)));
    assert!(importer.imports().is_empty());
    assert!(
        get_provider_message_mapping(&pool, provider_account_id, "gmail-block-raw")
            .await
            .expect("mapping lookup")
            .is_none()
    );
}

async fn account_error_class(pool: &sqlx::SqlitePool, provider_account_id: i64) -> Option<String> {
    sqlx::query_scalar("SELECT last_error_class FROM provider_accounts WHERE id = ?1")
        .bind(provider_account_id)
        .fetch_one(pool)
        .await
        .expect("account error class")
}

#[tokio::test]
async fn default_incremental_sync_tracks_inbox_message_additions_only() {
    let options = GmailIncrementalSyncOptions::into_mailboxes(["inbox"]);

    assert_eq!(options.history_types, vec!["messageAdded"]);
    assert_eq!(options.label_id.as_deref(), Some("INBOX"));
    assert_eq!(options.historical_fallback.label_ids, vec!["INBOX"]);
    assert_eq!(
        options.historical_fallback.target_mailbox_ids,
        vec!["inbox"]
    );
}

#[tokio::test]
async fn incremental_history_imports_new_messages_and_advances_cursor() {
    let (pool, _guard, user_id, provider_account_id) = setup(Some("100")).await;
    let gmail = FakeGmail::new(
        vec![Ok(history_page(
            vec![
                ("101", vec![("gmail-1", "thread-1")]),
                (
                    "102",
                    vec![("gmail-2", "thread-2"), ("gmail-1", "thread-1")],
                ),
            ],
            None,
            Some("103"),
        ))],
        Vec::new(),
        vec![
            raw_message("gmail-1", "thread-1", "101", "m1@example.com"),
            raw_message("gmail-2", "thread-2", "102", "m2@example.com"),
        ],
    );
    let importer = FakeRfc822Importer::default();
    let mut options = GmailIncrementalSyncOptions::into_mailboxes(["inbox"]);
    options.page_size = 50;

    let summary = run_gmail_incremental_sync(
        &pool,
        GmailIncrementalSyncAccount {
            provider_account_id,
            user_id,
            history_id: Some("100".to_owned()),
        },
        &gmail,
        &importer,
        options,
        &CancellationToken::new(),
    )
    .await
    .expect("incremental sync");

    assert_eq!(summary.history_records, 2);
    assert_eq!(summary.messages_seen, 2);
    assert_eq!(summary.imported, 2);
    assert!(summary.completed);
    assert_eq!(summary.end_history_id.as_deref(), Some("103"));
    assert_eq!(gmail.raw_gets(), vec!["gmail-1", "gmail-2"]);
    assert_eq!(gmail.history_params()[0].start_history_id, "100");

    let mapping = get_provider_message_mapping(&pool, provider_account_id, "gmail-2")
        .await
        .expect("mapping lookup")
        .expect("mapping exists");
    assert_eq!(mapping.import_status, ProviderImportStatus::Imported);
    assert_eq!(mapping.provider_history_id.as_deref(), Some("102"));

    let stored_cursor: Option<String> =
        sqlx::query_scalar("SELECT last_profile_history_id FROM provider_accounts WHERE id = ?1")
            .bind(provider_account_id)
            .fetch_one(&pool)
            .await
            .expect("cursor");
    assert_eq!(stored_cursor.as_deref(), Some("103"));

    let audit = list_provider_sync_audit_logs(&pool, user_id, provider_account_id, 20)
        .await
        .expect("audit logs");
    assert!(audit.iter().any(|log| log.event_type == "sync_started"));
    assert!(audit.iter().any(|log| log.event_type == "sync_completed"));
    assert_eq!(
        audit
            .iter()
            .filter(|log| log.event_type == "message_imported")
            .count(),
        2
    );
}

#[tokio::test]
async fn incremental_history_imports_gmail_user_labels_to_local_thread_labels() {
    let (pool, _guard, user_id, provider_account_id) = setup(Some("100")).await;
    let gmail = FakeGmail::with_labels(
        vec![Ok(history_page(
            vec![(
                "101",
                vec![("gmail-incremental-label", "gmail-thread-label")],
            )],
            None,
            Some("102"),
        ))],
        Vec::new(),
        vec![raw_message_with_labels(
            "gmail-incremental-label",
            "gmail-thread-label",
            "101",
            "incremental-label@example.com",
            &["Label_Inc", "SPAM", "CATEGORY_UPDATES"],
        )],
        vec![
            gmail_label("Label_Inc", "Projects/Incremental", "user"),
            gmail_label("SPAM", "Spam", "system"),
            gmail_label("CATEGORY_UPDATES", "Updates", "system"),
        ],
    );
    let importer = FakeRfc822Importer::default();

    let summary = run_gmail_incremental_sync(
        &pool,
        GmailIncrementalSyncAccount {
            provider_account_id,
            user_id,
            history_id: Some("100".to_owned()),
        },
        &gmail,
        &importer,
        GmailIncrementalSyncOptions::into_mailboxes(["inbox"]),
        &CancellationToken::new(),
    )
    .await
    .expect("incremental sync");

    assert_eq!(summary.imported, 1);
    assert_eq!(
        thread_label_names(&pool, user_id, "thread-1").await,
        vec!["Projects/Incremental".to_owned()]
    );
}

#[tokio::test]
async fn incremental_duplicate_still_imports_newly_seen_gmail_labels_to_existing_thread() {
    let (pool, _guard, user_id, provider_account_id) = setup(Some("100")).await;
    let importer = FakeRfc822Importer::default();
    let options = GmailIncrementalSyncOptions::into_mailboxes(["inbox"]);

    let first_gmail = FakeGmail::new(
        vec![Ok(history_page(
            vec![(
                "101",
                vec![("gmail-label-dupe-a", "gmail-thread-label-dupe")],
            )],
            None,
            Some("102"),
        ))],
        Vec::new(),
        vec![raw_message(
            "gmail-label-dupe-a",
            "gmail-thread-label-dupe",
            "101",
            "incremental-label-dupe@example.com",
        )],
    );
    run_gmail_incremental_sync(
        &pool,
        GmailIncrementalSyncAccount {
            provider_account_id,
            user_id,
            history_id: Some("100".to_owned()),
        },
        &first_gmail,
        &importer,
        options.clone(),
        &CancellationToken::new(),
    )
    .await
    .expect("first incremental sync");

    let second_gmail = FakeGmail::with_labels(
        vec![Ok(history_page(
            vec![(
                "103",
                vec![("gmail-label-dupe-b", "gmail-thread-label-dupe")],
            )],
            None,
            Some("104"),
        ))],
        Vec::new(),
        vec![raw_message_with_labels(
            "gmail-label-dupe-b",
            "gmail-thread-label-dupe",
            "103",
            "incremental-label-dupe@example.com",
            &["Label_Dupe"],
        )],
        vec![gmail_label("Label_Dupe", "Projects/Duplicate", "user")],
    );
    let second = run_gmail_incremental_sync(
        &pool,
        GmailIncrementalSyncAccount {
            provider_account_id,
            user_id,
            history_id: Some("102".to_owned()),
        },
        &second_gmail,
        &importer,
        options,
        &CancellationToken::new(),
    )
    .await
    .expect("second incremental sync");

    assert_eq!(second.imported, 0);
    assert_eq!(second.duplicates, 1);
    assert_eq!(importer.local_message_count(), 1);
    assert_eq!(
        thread_label_names(&pool, user_id, "thread-1").await,
        vec!["Projects/Duplicate".to_owned()]
    );
}

#[tokio::test]
async fn incremental_label_metadata_rename_does_not_remove_absent_later_label() {
    let (pool, _guard, user_id, provider_account_id) = setup(Some("100")).await;
    let importer = FakeRfc822Importer::default();
    let options = GmailIncrementalSyncOptions::into_mailboxes(["inbox"]);

    let first_gmail = FakeGmail::with_labels(
        vec![Ok(history_page(
            vec![(
                "101",
                vec![("gmail-inc-label-rename-a", "gmail-thread-inc-label")],
            )],
            None,
            Some("102"),
        ))],
        Vec::new(),
        vec![raw_message_with_labels(
            "gmail-inc-label-rename-a",
            "gmail-thread-inc-label",
            "101",
            "inc-label-rename-a@example.com",
            &["Label_Status"],
        )],
        vec![gmail_label("Label_Status", "Status/Old", "user")],
    );
    let first = run_gmail_incremental_sync(
        &pool,
        GmailIncrementalSyncAccount {
            provider_account_id,
            user_id,
            history_id: Some("100".to_owned()),
        },
        &first_gmail,
        &importer,
        options.clone(),
        &CancellationToken::new(),
    )
    .await
    .expect("first incremental sync");
    assert_eq!(first.imported, 1);
    assert_eq!(
        thread_label_names(&pool, user_id, "thread-1").await,
        vec!["Status/Old".to_owned()]
    );

    let rename_gmail = FakeGmail::with_labels(
        vec![Ok(history_page(
            vec![(
                "103",
                vec![("gmail-inc-label-rename-b", "gmail-thread-inc-label")],
            )],
            None,
            Some("104"),
        ))],
        Vec::new(),
        vec![raw_message_with_labels(
            "gmail-inc-label-rename-b",
            "gmail-thread-inc-label",
            "103",
            "inc-label-rename-b@example.com",
            &["Label_Status"],
        )],
        vec![gmail_label("Label_Status", "Status/New", "user")],
    );
    let renamed = run_gmail_incremental_sync(
        &pool,
        GmailIncrementalSyncAccount {
            provider_account_id,
            user_id,
            history_id: Some("102".to_owned()),
        },
        &rename_gmail,
        &importer,
        options.clone(),
        &CancellationToken::new(),
    )
    .await
    .expect("rename incremental sync");
    assert_eq!(renamed.imported, 1);
    assert_eq!(
        thread_label_names(&pool, user_id, "thread-2").await,
        vec!["Status/New".to_owned()]
    );

    let removed_later_gmail = FakeGmail::with_labels(
        vec![Ok(history_page(
            vec![(
                "105",
                vec![("gmail-inc-label-rename-c", "gmail-thread-inc-label")],
            )],
            None,
            Some("106"),
        ))],
        Vec::new(),
        vec![raw_message_with_labels(
            "gmail-inc-label-rename-c",
            "gmail-thread-inc-label",
            "105",
            "inc-label-rename-b@example.com",
            &[],
        )],
        Vec::new(),
    );
    let removed = run_gmail_incremental_sync(
        &pool,
        GmailIncrementalSyncAccount {
            provider_account_id,
            user_id,
            history_id: Some("104".to_owned()),
        },
        &removed_later_gmail,
        &importer,
        options,
        &CancellationToken::new(),
    )
    .await
    .expect("absent-label incremental sync");

    assert_eq!(removed.imported, 0);
    assert_eq!(removed.duplicates, 1);
    assert_eq!(
        thread_label_names(&pool, user_id, "thread-2").await,
        vec!["Status/New".to_owned()],
        "Gmail label absence/removal in later history must not remove a local thread label"
    );
}

#[tokio::test]
async fn expired_history_cursor_runs_bounded_full_sync_and_audits_fallback() {
    let (pool, _guard, user_id, provider_account_id) = setup(Some("expired-100")).await;
    let fallback = gmail_import_fixture(GmailImportScenario::ExpiredCursorFallback);
    let gmail = FakeGmail::new(
        vec![Err(GmailClientError::Api {
            status: StatusCode::NOT_FOUND,
            kind: GmailApiErrorKind::NotFound,
            reason: Some("notFound".to_owned()),
            message: "HistoryId not found".to_owned(),
            retry_after: None,
        })],
        vec![message_page_from_fixtures([fallback])],
        vec![raw_fixture_message(fallback)],
    );
    let importer = FakeRfc822Importer::default();
    let mut options = GmailIncrementalSyncOptions::into_mailboxes(["inbox"]);
    options.historical_fallback.max_messages = Some(10);

    let summary = run_gmail_incremental_sync(
        &pool,
        GmailIncrementalSyncAccount {
            provider_account_id,
            user_id,
            history_id: Some("expired-100".to_owned()),
        },
        &gmail,
        &importer,
        options,
        &CancellationToken::new(),
    )
    .await
    .expect("fallback sync");

    assert!(summary.fallback_full_sync);
    assert_eq!(summary.imported, 1);
    assert_eq!(summary.end_history_id.as_deref(), Some(fallback.history_id));

    let stored_cursor: Option<String> =
        sqlx::query_scalar("SELECT last_profile_history_id FROM provider_accounts WHERE id = ?1")
            .bind(provider_account_id)
            .fetch_one(&pool)
            .await
            .expect("cursor");
    assert_eq!(stored_cursor.as_deref(), Some(fallback.history_id));

    let mapping = get_provider_message_mapping(&pool, provider_account_id, fallback.gmail_id)
        .await
        .expect("mapping lookup")
        .expect("fallback mapping");
    assert_eq!(mapping.import_status, ProviderImportStatus::Imported);
    assert_eq!(
        mapping.rfc822_message_id.as_deref(),
        Some(fallback.rfc822_message_id)
    );
    assert_eq!(gmail.raw_gets(), vec![fallback.gmail_id]);
    assert_eq!(importer.imports()[0].raw_rfc822, fallback.raw_rfc822());

    let audit = list_provider_sync_audit_logs(&pool, user_id, provider_account_id, 20)
        .await
        .expect("audit logs");
    assert!(audit.iter().any(|log| {
        log.event_type == "sync_failed"
            && log.safe_error_class.as_deref() == Some("gmail_history_cursor_expired")
    }));
    assert!(audit.iter().any(|log| log.event_type == "sync_completed"));
}

#[tokio::test]
async fn rerunning_same_incremental_history_page_does_not_duplicate_stalwart_mail() {
    let (pool, _guard, user_id, provider_account_id) = setup(Some("100")).await;
    let importer = FakeRfc822Importer::default();
    let options = GmailIncrementalSyncOptions::into_mailboxes(["inbox"]);

    let first_gmail = FakeGmail::new(
        vec![Ok(history_page(
            vec![("101", vec![("gmail-incremental-idem", "thread-idem")])],
            None,
            Some("102"),
        ))],
        Vec::new(),
        vec![raw_message(
            "gmail-incremental-idem",
            "thread-idem",
            "101",
            "incremental-idem@example.com",
        )],
    );
    let first = run_gmail_incremental_sync(
        &pool,
        GmailIncrementalSyncAccount {
            provider_account_id,
            user_id,
            history_id: Some("100".to_owned()),
        },
        &first_gmail,
        &importer,
        options.clone(),
        &CancellationToken::new(),
    )
    .await
    .expect("first incremental sync");
    assert_eq!(first.messages_seen, 1);
    assert_eq!(first.imported, 1);
    assert_eq!(importer.imports().len(), 1);

    let rerun_gmail = FakeGmail::new(
        vec![Ok(history_page(
            vec![("101", vec![("gmail-incremental-idem", "thread-idem")])],
            None,
            Some("102"),
        ))],
        Vec::new(),
        Vec::new(),
    );
    let rerun = run_gmail_incremental_sync(
        &pool,
        GmailIncrementalSyncAccount {
            provider_account_id,
            user_id,
            history_id: Some("100".to_owned()),
        },
        &rerun_gmail,
        &importer,
        options,
        &CancellationToken::new(),
    )
    .await
    .expect("rerun incremental sync");

    assert_eq!(rerun.messages_seen, 1);
    assert_eq!(rerun.skipped, 1);
    assert_eq!(rerun.imported, 0);
    assert!(rerun_gmail.raw_gets().is_empty());
    assert_eq!(importer.imports().len(), 1);
}

#[tokio::test]
async fn retry_after_incremental_crash_after_local_import_advances_cursor_without_duplicate_mail() {
    let (pool, _guard, user_id, provider_account_id) = setup(Some("100")).await;
    let importer = FakeRfc822Importer::default();
    let crashing_importer = PostImportCrashImporter::new(&importer, "gmail-incremental-crash");
    let options = GmailIncrementalSyncOptions::into_mailboxes(["inbox"]);

    let first_gmail = FakeGmail::new(
        vec![Ok(history_page(
            vec![(
                "101",
                vec![("gmail-incremental-crash", "thread-incremental-crash")],
            )],
            None,
            Some("102"),
        ))],
        Vec::new(),
        vec![raw_message(
            "gmail-incremental-crash",
            "thread-incremental-crash",
            "101",
            "incremental-crash@example.com",
        )],
    );
    let first = run_gmail_incremental_sync(
        &pool,
        GmailIncrementalSyncAccount {
            provider_account_id,
            user_id,
            history_id: Some("100".to_owned()),
        },
        &first_gmail,
        &crashing_importer,
        options.clone(),
        &CancellationToken::new(),
    )
    .await
    .expect("first incremental pass records failed post-import window");

    assert_eq!(first.messages_seen, 1);
    assert_eq!(first.failed, 1);
    assert_eq!(first.imported, 0);
    assert_eq!(
        profile_history_cursor(&pool, provider_account_id)
            .await
            .as_deref(),
        Some("102")
    );
    assert_eq!(importer.local_message_count(), 1);
    let failed =
        get_provider_message_mapping(&pool, provider_account_id, "gmail-incremental-crash")
            .await
            .expect("failed mapping lookup")
            .expect("failed mapping");
    assert_eq!(failed.import_status, ProviderImportStatus::Failed);
    assert_eq!(failed.error_class.as_deref(), Some("stalwart_import"));
    assert!(failed.jmap_email_id.is_none());

    let retry_gmail = FakeGmail::new(
        vec![Ok(history_page(
            vec![(
                "101",
                vec![("gmail-incremental-crash", "thread-incremental-crash")],
            )],
            None,
            Some("102"),
        ))],
        Vec::new(),
        vec![raw_message(
            "gmail-incremental-crash",
            "thread-incremental-crash",
            "101-retry",
            "incremental-crash@example.com",
        )],
    );
    let retry = run_gmail_incremental_sync(
        &pool,
        GmailIncrementalSyncAccount {
            provider_account_id,
            user_id,
            history_id: Some("100".to_owned()),
        },
        &retry_gmail,
        &importer,
        options,
        &CancellationToken::new(),
    )
    .await
    .expect("retry same incremental history page");

    assert_eq!(retry.messages_seen, 1);
    assert_eq!(retry.failed, 0);
    assert_eq!(retry.imported, 0);
    assert_eq!(retry.duplicates, 1);
    assert_eq!(
        profile_history_cursor(&pool, provider_account_id)
            .await
            .as_deref(),
        Some("102")
    );
    assert_eq!(importer.local_message_count(), 1);
    let mapping =
        get_provider_message_mapping(&pool, provider_account_id, "gmail-incremental-crash")
            .await
            .expect("mapping lookup")
            .expect("mapping converged");
    assert_eq!(mapping.import_status, ProviderImportStatus::Duplicate);
    assert_eq!(mapping.jmap_email_id.as_deref(), Some("email-1"));
    assert_eq!(
        mapping.rfc822_message_id.as_deref(),
        Some("incremental-crash@example.com")
    );
    assert!(mapping.error_class.is_none());

    let audit = list_provider_sync_audit_logs(&pool, user_id, provider_account_id, 20)
        .await
        .expect("audit logs");
    assert!(audit.iter().any(|log| {
        log.event_type == "message_failed"
            && log.provider_message_id.as_deref() == Some("gmail-incremental-crash")
            && log.safe_error_class.as_deref() == Some("stalwart_import")
    }));
    assert!(audit.iter().any(|log| {
        log.event_type == "message_imported"
            && log.provider_message_id.as_deref() == Some("gmail-incremental-crash")
            && log
                .metadata_json
                .as_deref()
                .is_some_and(|metadata| metadata.contains("\"duplicate\":true"))
    }));
}
