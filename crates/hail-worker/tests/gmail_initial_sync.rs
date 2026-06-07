use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use hail_worker::gmail_client::{
    GmailClientError, GmailProfile, ListMessage, ListMessagesParams, ListMessagesResponse,
    RawGmailMessage,
};
use hail_worker::gmail_historical_import::GmailHistoricalSource;
use hail_worker::gmail_initial_sync::{
    GmailInitialSyncOptions, GmailInitialSyncSource, GmailProviderAccount,
    load_gmail_provider_account, run_gmail_initial_sync,
};
use hail_worker::rfc822_import::FakeRfc822Importer;
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
            "hail-worker-gmail-initial-sync-test-{pid}-{nanos}-{attempt}"
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

async fn setup() -> (sqlx::SqlitePool, TempDb, i64, GmailProviderAccount) {
    let (url, guard) = fresh_db_url();
    let pool = hail_db::connect(&url).await.expect("connect");
    hail_db::migrate(&pool).await.expect("migrate");
    let user_id = insert_user(&pool, "importer@example.com", "acct-importer").await;
    let provider_row_id = insert_provider_account(&pool, user_id, "acct-importer").await;
    let account = load_gmail_provider_account(&pool, provider_row_id)
        .await
        .expect("load mail account")
        .expect("mail account exists");
    (pool, guard, user_id, account)
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
    jmap_account_id: &str,
) -> i64 {
    sqlx::query(
        "INSERT INTO mail_accounts \
         (user_id, jmap_account_id, backend_kind, provider_kind, provider_account_id, provider_email, \
          refresh_token_enc, sync_status, created_at, updated_at) \
         VALUES (?, ?, 'gmail', 'gmail', ?, ?, ?, 'active', ?, ?)",
    )
    .bind(user_id)
    .bind(jmap_account_id)
    .bind("gmail-user@example.com")
    .bind("Gmail.User@Example.COM")
    .bind(vec![1_u8; 29])
    .bind("2026-01-01T00:00:00Z")
    .bind("2026-01-01T00:00:00Z")
    .execute(pool)
    .await
    .expect("mail account insert");

    sqlx::query_scalar("SELECT id FROM mail_accounts WHERE user_id = ?")
        .bind(user_id)
        .fetch_one(pool)
        .await
        .expect("mail account id")
}

#[derive(Clone, Debug)]
struct FakeGmail {
    profile: GmailProfile,
    pages: Arc<Mutex<Vec<ListMessagesResponse>>>,
    raw: Arc<HashMap<String, RawGmailMessage>>,
    list_params: Arc<Mutex<Vec<ListMessagesParams>>>,
    raw_gets: Arc<Mutex<Vec<String>>>,
    profile_calls: Arc<Mutex<usize>>,
    user_label_calls: Arc<Mutex<usize>>,
}

impl FakeGmail {
    fn new(
        profile: GmailProfile,
        pages: Vec<ListMessagesResponse>,
        raw: Vec<RawGmailMessage>,
    ) -> Self {
        Self {
            profile,
            pages: Arc::new(Mutex::new(pages)),
            raw: Arc::new(
                raw.into_iter()
                    .map(|message| (message.id.clone(), message))
                    .collect(),
            ),
            list_params: Arc::new(Mutex::new(Vec::new())),
            raw_gets: Arc::new(Mutex::new(Vec::new())),
            profile_calls: Arc::new(Mutex::new(0)),
            user_label_calls: Arc::new(Mutex::new(0)),
        }
    }

    fn list_params(&self) -> Vec<ListMessagesParams> {
        self.list_params.lock().expect("list_params").clone()
    }

    fn raw_gets(&self) -> Vec<String> {
        self.raw_gets.lock().expect("raw_gets").clone()
    }

    fn profile_calls(&self) -> usize {
        *self.profile_calls.lock().expect("profile_calls")
    }

    fn user_label_calls(&self) -> usize {
        *self.user_label_calls.lock().expect("user_label_calls")
    }
}

#[async_trait]
impl GmailInitialSyncSource for FakeGmail {
    async fn profile(&self) -> Result<GmailProfile, GmailClientError> {
        *self.profile_calls.lock().expect("profile_calls") += 1;
        Ok(self.profile.clone())
    }
}

#[async_trait]
impl GmailHistoricalSource for FakeGmail {
    async fn list_messages(
        &self,
        params: &ListMessagesParams,
    ) -> Result<ListMessagesResponse, GmailClientError> {
        self.list_params
            .lock()
            .expect("list_params")
            .push(params.clone());
        Ok(self.pages.lock().expect("pages").remove(0))
    }

    async fn get_raw_message(&self, message_id: &str) -> Result<RawGmailMessage, GmailClientError> {
        self.raw_gets
            .lock()
            .expect("raw_gets")
            .push(message_id.to_owned());
        self.raw
            .get(message_id)
            .cloned()
            .ok_or(GmailClientError::MissingRawMessage)
    }

    async fn list_user_labels(
        &self,
    ) -> Result<Vec<hail_worker::gmail_client::GmailLabel>, GmailClientError> {
        *self.user_label_calls.lock().expect("user_label_calls") += 1;
        Ok(Vec::new())
    }
}

fn profile(email: &str, history_id: Option<&str>) -> GmailProfile {
    GmailProfile {
        email_address: email.to_owned(),
        messages_total: Some(2),
        threads_total: Some(2),
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

#[tokio::test]
async fn initial_sync_default_options_import_only_gmail_inbox_label() {
    let options = GmailInitialSyncOptions::into_mailboxes(["inbox"]);

    assert_eq!(options.historical.label_ids, vec!["INBOX"]);
    assert_eq!(options.historical.target_mailbox_ids, vec!["inbox"]);
}

#[tokio::test]
async fn initial_sync_verifies_profile_persists_history_id_and_imports_bounded_messages() {
    let (pool, _guard, _user_id, account) = setup().await;
    let gmail = FakeGmail::new(
        profile("gmail.user@example.com", Some("profile-history-42")),
        vec![ListMessagesResponse {
            messages: vec![
                ListMessage {
                    id: "gmail-1".to_owned(),
                    thread_id: Some("thread-1".to_owned()),
                },
                ListMessage {
                    id: "gmail-2".to_owned(),
                    thread_id: Some("thread-2".to_owned()),
                },
            ],
            next_page_token: Some("next-page".to_owned()),
            result_size_estimate: Some(10),
        }],
        vec![
            raw_message("gmail-1", "thread-1", "history-1", "m1@example.com"),
            raw_message("gmail-2", "thread-2", "history-2", "m2@example.com"),
        ],
    );
    let importer = FakeRfc822Importer::default();
    let mut options = GmailInitialSyncOptions::into_mailboxes(["inbox"]);
    options.historical.max_messages = Some(2);
    options.historical.page_size = 50;

    let summary = run_gmail_initial_sync(
        &pool,
        account.clone(),
        &gmail,
        &importer,
        options,
        &CancellationToken::new(),
    )
    .await
    .expect("initial sync");

    assert_eq!(gmail.profile_calls(), 1);
    assert_eq!(gmail.user_label_calls(), 1);
    assert_eq!(
        summary.profile.history_id.as_deref(),
        Some("profile-history-42")
    );
    assert_eq!(summary.import.listed, 2);
    assert_eq!(summary.import.imported, 2);
    assert!(!summary.import.completed);
    assert!(summary.import.bounded);
    assert_eq!(summary.import.bound_max_messages, Some(2));
    assert_eq!(gmail.raw_gets(), vec!["gmail-1", "gmail-2"]);
    assert_eq!(gmail.list_params()[0].label_ids, vec!["INBOX"]);
    assert_eq!(gmail.list_params()[0].query.as_deref(), Some("-in:sent"));
    assert!(!gmail.list_params()[0].include_spam_trash);
    assert_eq!(importer.imports().len(), 2);

    let row: (String, String, Option<String>, Option<String>, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT sync_status, last_profile_history_id, profile_synced_at, initial_sync_completed_at, backfill_cursor_json, last_error_message \
         FROM mail_accounts WHERE id = ?1",
    )
    .bind(account.id)
    .fetch_one(&pool)
    .await
    .expect("mail account row");
    assert_eq!(row.0, "initial_sync");
    assert_eq!(row.1, "profile-history-42");
    assert!(row.2.is_some());
    assert!(row.3.is_none());
    assert!(row.4.expect("cursor").contains("next-page"));
    assert!(row.5.is_none());

    let audit = hail_db::provider_sync_audit::list_provider_sync_audit_logs(
        &pool,
        account.user_id,
        account.id,
        20,
    )
    .await
    .expect("audit logs");
    assert!(audit.iter().any(|log| {
        log.event_type == "message_skipped"
            && log.safe_error_class.as_deref() == Some("configured_initial_import_bound")
    }));
}

#[tokio::test]
async fn initial_sync_rejects_wrong_gmail_profile_before_listing_messages() {
    let (pool, _guard, _user_id, account) = setup().await;
    let gmail = FakeGmail::new(
        profile("other@example.com", Some("999")),
        Vec::new(),
        Vec::new(),
    );
    let importer = FakeRfc822Importer::default();

    let err = run_gmail_initial_sync(
        &pool,
        account.clone(),
        &gmail,
        &importer,
        GmailInitialSyncOptions::into_mailboxes(["inbox"]),
        &CancellationToken::new(),
    )
    .await
    .expect_err("profile mismatch");

    assert!(err.to_string().contains("does not match mail account"));
    assert!(gmail.list_params().is_empty());
    assert!(gmail.raw_gets().is_empty());
    assert!(importer.imports().is_empty());

    let row: (String, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT sync_status, last_profile_history_id, last_error_class \
         FROM mail_accounts WHERE id = ?1",
    )
    .bind(account.id)
    .fetch_one(&pool)
    .await
    .expect("mail account row");
    assert_eq!(row.0, "error");
    assert!(row.1.is_none());
    assert_eq!(row.2.as_deref(), Some("gmail_profile_mismatch"));
}
