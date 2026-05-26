use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use hail_db::provider_message_mappings::{ProviderImportStatus, get_provider_message_mapping};
use hail_db::provider_sync_audit::list_provider_sync_audit_logs;
use hail_worker::gmail_client::{
    GmailApiErrorKind, GmailClientError, GmailHistoryMessage, GmailHistoryMessageRef,
    GmailHistoryRecord, ListHistoryParams, ListHistoryResponse, ListMessage, ListMessagesParams,
    ListMessagesResponse, RawGmailMessage,
};
use hail_worker::gmail_historical_import::GmailHistoricalSource;
use hail_worker::gmail_incremental_sync::{
    GmailIncrementalSource, GmailIncrementalSyncAccount, GmailIncrementalSyncOptions,
    run_gmail_incremental_sync,
};
use hail_worker::rfc822_import::FakeRfc822Importer;
use reqwest::StatusCode;
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
          refresh_token_ref, last_profile_history_id, sync_status, created_at, updated_at) \
         VALUES (?, 'acct-sync', 'gmail', ?, ?, 'kms://hail/provider-token/1', ?, 'active', ?, ?)",
    )
    .bind(user_id)
    .bind(format!("gmail-provider-{user_id}"))
    .bind(format!("user-{user_id}@gmail.example"))
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
    history_params: Arc<Mutex<Vec<ListHistoryParams>>>,
    raw_gets: Arc<Mutex<Vec<String>>>,
}

impl FakeGmail {
    fn new(
        history_pages: Vec<Result<ListHistoryResponse, GmailClientError>>,
        message_pages: Vec<ListMessagesResponse>,
        raw: Vec<RawGmailMessage>,
    ) -> Self {
        Self {
            history_pages: Arc::new(Mutex::new(history_pages)),
            message_pages: Arc::new(Mutex::new(message_pages)),
            raw: Arc::new(raw.into_iter().map(|msg| (msg.id.clone(), msg)).collect()),
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

fn message_page(ids: Vec<(&str, &str)>) -> ListMessagesResponse {
    ListMessagesResponse {
        messages: ids
            .into_iter()
            .map(|(id, thread_id)| ListMessage {
                id: id.to_owned(),
                thread_id: Some(thread_id.to_owned()),
            })
            .collect(),
        next_page_token: None,
        result_size_estimate: None,
    }
}

fn raw_message(id: &str, thread_id: &str, history_id: &str, message_id: &str) -> RawGmailMessage {
    RawGmailMessage {
        id: id.to_owned(),
        thread_id: Some(thread_id.to_owned()),
        history_id: Some(history_id.to_owned()),
        rfc822: format!(
            "From: sender@example.com\r\nTo: user@example.com\r\nMessage-ID: <{message_id}>\r\nSubject: hi\r\n\r\nBody"
        )
        .into_bytes(),
    }
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
async fn expired_history_cursor_runs_bounded_full_sync_and_audits_fallback() {
    let (pool, _guard, user_id, provider_account_id) = setup(Some("expired-100")).await;
    let gmail = FakeGmail::new(
        vec![Err(GmailClientError::Api {
            status: StatusCode::NOT_FOUND,
            kind: GmailApiErrorKind::NotFound,
            reason: Some("notFound".to_owned()),
            message: "HistoryId not found".to_owned(),
            retry_after: None,
        })],
        vec![message_page(vec![("gmail-fallback", "thread-fallback")])],
        vec![raw_message(
            "gmail-fallback",
            "thread-fallback",
            "250",
            "fallback@example.com",
        )],
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
    assert_eq!(summary.end_history_id.as_deref(), Some("250"));

    let stored_cursor: Option<String> =
        sqlx::query_scalar("SELECT last_profile_history_id FROM provider_accounts WHERE id = ?1")
            .bind(provider_account_id)
            .fetch_one(&pool)
            .await
            .expect("cursor");
    assert_eq!(stored_cursor.as_deref(), Some("250"));

    let audit = list_provider_sync_audit_logs(&pool, user_id, provider_account_id, 20)
        .await
        .expect("audit logs");
    assert!(audit.iter().any(|log| {
        log.event_type == "sync_failed"
            && log.safe_error_class.as_deref() == Some("gmail_history_cursor_expired")
    }));
    assert!(audit.iter().any(|log| log.event_type == "sync_completed"));
}
