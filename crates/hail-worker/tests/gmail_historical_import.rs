use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use hail_db::provider_message_mappings::{
    ImportedProviderMessageMapping, ProviderImportStatus, get_provider_message_mapping,
    mark_provider_message_imported,
};
use hail_db::provider_sync_audit::list_provider_sync_audit_logs;
use hail_worker::gmail_client::{
    GmailClientError, ListMessage, ListMessagesParams, ListMessagesResponse, RawGmailMessage,
};
use hail_worker::gmail_historical_import::{
    GmailHistoricalImportAccount, GmailHistoricalImportOptions, GmailHistoricalSource,
    import_gmail_history,
};
use hail_worker::rfc822_import::{FakeRfc822Importer, Rfc822ImportError};
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
            "hail-worker-gmail-historical-import-test-{pid}-{nanos}-{attempt}"
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

async fn setup() -> (sqlx::SqlitePool, TempDb, i64, i64) {
    let (url, guard) = fresh_db_url();
    let pool = hail_db::connect(&url).await.expect("connect");
    hail_db::migrate(&pool).await.expect("migrate");
    let user_id = insert_user(&pool, "importer@example.com", "acct-importer").await;
    let provider_account_id = insert_provider_account(&pool, user_id, "acct-importer").await;
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
    jmap_account_id: &str,
) -> i64 {
    sqlx::query(
        "INSERT INTO provider_accounts \
         (user_id, jmap_account_id, provider_kind, provider_account_id, provider_email, \
          refresh_token_ref, sync_status, created_at, updated_at) \
         VALUES (?, ?, 'gmail', ?, ?, 'kms://hail/provider-token/1', 'active', ?, ?)",
    )
    .bind(user_id)
    .bind(jmap_account_id)
    .bind(format!("gmail-provider-{user_id}"))
    .bind(format!("user-{user_id}@gmail.example"))
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
    pages: Arc<Mutex<Vec<ListMessagesResponse>>>,
    raw: Arc<HashMap<String, RawGmailMessage>>,
    list_params: Arc<Mutex<Vec<ListMessagesParams>>>,
    raw_gets: Arc<Mutex<Vec<String>>>,
}

impl FakeGmail {
    fn new(pages: Vec<ListMessagesResponse>, raw: Vec<RawGmailMessage>) -> Self {
        Self {
            pages: Arc::new(Mutex::new(pages)),
            raw: Arc::new(
                raw.into_iter()
                    .map(|message| (message.id.clone(), message))
                    .collect(),
            ),
            list_params: Arc::new(Mutex::new(Vec::new())),
            raw_gets: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn raw_gets(&self) -> Vec<String> {
        self.raw_gets.lock().expect("raw_gets").clone()
    }

    fn list_params(&self) -> Vec<ListMessagesParams> {
        self.list_params.lock().expect("list_params").clone()
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
async fn imports_gmail_pages_into_stalwart_and_records_mapping_and_audit() {
    let (pool, _guard, user_id, provider_account_id) = setup().await;
    let gmail = FakeGmail::new(
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
            next_page_token: None,
            result_size_estimate: Some(2),
        }],
        vec![
            raw_message("gmail-1", "thread-1", "history-1", "m1@example.com"),
            raw_message("gmail-2", "thread-2", "history-2", "m2@example.com"),
        ],
    );
    let importer = FakeRfc822Importer::default();

    let mut options = GmailHistoricalImportOptions::into_mailboxes(["inbox"]);
    options.label_ids = vec!["INBOX".to_owned()];
    options.query = Some("newer_than:30d".to_owned());
    options.max_messages = Some(10);
    options.page_size = 50;
    options.keywords = vec!["$seen".to_owned()];

    let summary = import_gmail_history(
        &pool,
        GmailHistoricalImportAccount {
            provider_account_id,
            user_id,
        },
        &gmail,
        &importer,
        options,
        &CancellationToken::new(),
    )
    .await
    .expect("historical import");

    assert_eq!(summary.listed, 2);
    assert_eq!(summary.fetched, 2);
    assert_eq!(summary.imported, 2);
    assert_eq!(summary.duplicates, 0);
    assert!(summary.completed);

    let first = get_provider_message_mapping(&pool, provider_account_id, "gmail-1")
        .await
        .expect("mapping lookup")
        .expect("mapping exists");
    assert_eq!(first.import_status, ProviderImportStatus::Imported);
    assert_eq!(first.provider_thread_id.as_deref(), Some("thread-1"));
    assert_eq!(first.provider_history_id.as_deref(), Some("history-1"));
    assert_eq!(first.rfc822_message_id.as_deref(), Some("m1@example.com"));
    assert_eq!(first.jmap_email_id.as_deref(), Some("email-1"));

    let imports = importer.imports();
    assert_eq!(imports.len(), 2);
    assert_eq!(imports[0].mailbox_ids, vec!["inbox"]);
    assert_eq!(imports[0].keywords, vec!["$seen"]);

    let audit = list_provider_sync_audit_logs(&pool, user_id, provider_account_id, 10)
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
async fn rerun_skips_existing_provider_mapping_without_fetching_raw() {
    let (pool, _guard, user_id, provider_account_id) = setup().await;
    mark_provider_message_imported(
        &pool,
        ImportedProviderMessageMapping {
            provider_account_id,
            provider_message_id: "gmail-existing",
            provider_thread_id: Some("thread-existing"),
            provider_history_id: Some("history-existing"),
            rfc822_message_id: Some("existing@example.com"),
            content_sha256: None,
            jmap_email_id: "local-existing",
            jmap_thread_id: Some("local-thread"),
            jmap_mailbox_ids_json: Some(r#"["inbox"]"#),
        },
    )
    .await
    .expect("seed mapping");
    let gmail = FakeGmail::new(
        vec![ListMessagesResponse {
            messages: vec![ListMessage {
                id: "gmail-existing".to_owned(),
                thread_id: Some("thread-existing".to_owned()),
            }],
            next_page_token: None,
            result_size_estimate: Some(1),
        }],
        Vec::new(),
    );
    let importer = FakeRfc822Importer::default();

    let summary = import_gmail_history(
        &pool,
        GmailHistoricalImportAccount {
            provider_account_id,
            user_id,
        },
        &gmail,
        &importer,
        GmailHistoricalImportOptions::into_mailboxes(["inbox"]),
        &CancellationToken::new(),
    )
    .await
    .expect("historical import rerun");

    assert_eq!(summary.skipped, 1);
    assert_eq!(summary.fetched, 0);
    assert!(gmail.raw_gets().is_empty());
    assert!(importer.imports().is_empty());
}

#[tokio::test]
async fn dedupes_different_provider_id_by_rfc822_message_id() {
    let (pool, _guard, user_id, provider_account_id) = setup().await;
    mark_provider_message_imported(
        &pool,
        ImportedProviderMessageMapping {
            provider_account_id,
            provider_message_id: "gmail-original",
            provider_thread_id: Some("thread-original"),
            provider_history_id: Some("history-original"),
            rfc822_message_id: Some("same@example.com"),
            content_sha256: None,
            jmap_email_id: "local-original",
            jmap_thread_id: Some("local-thread"),
            jmap_mailbox_ids_json: Some(r#"["inbox"]"#),
        },
    )
    .await
    .expect("seed mapping");
    let gmail = FakeGmail::new(
        vec![ListMessagesResponse {
            messages: vec![ListMessage {
                id: "gmail-copy".to_owned(),
                thread_id: Some("thread-copy".to_owned()),
            }],
            next_page_token: None,
            result_size_estimate: Some(1),
        }],
        vec![raw_message(
            "gmail-copy",
            "thread-copy",
            "history-copy",
            "same@example.com",
        )],
    );
    let importer = FakeRfc822Importer::default();

    let summary = import_gmail_history(
        &pool,
        GmailHistoricalImportAccount {
            provider_account_id,
            user_id,
        },
        &gmail,
        &importer,
        GmailHistoricalImportOptions::into_mailboxes(["inbox"]),
        &CancellationToken::new(),
    )
    .await
    .expect("historical import duplicate");

    assert_eq!(summary.fetched, 1);
    assert_eq!(summary.duplicates, 1);
    assert!(importer.imports().is_empty());
    let copy = get_provider_message_mapping(&pool, provider_account_id, "gmail-copy")
        .await
        .expect("mapping lookup")
        .expect("copy mapping");
    assert_eq!(copy.import_status, ProviderImportStatus::Duplicate);
    assert_eq!(copy.jmap_email_id.as_deref(), Some("local-original"));
}

#[tokio::test]
async fn bounded_import_saves_resume_cursor_and_uses_it_next_run() {
    let (pool, _guard, user_id, provider_account_id) = setup().await;
    let first_gmail = FakeGmail::new(
        vec![ListMessagesResponse {
            messages: vec![ListMessage {
                id: "gmail-1".to_owned(),
                thread_id: Some("thread-1".to_owned()),
            }],
            next_page_token: Some("page-2".to_owned()),
            result_size_estimate: Some(2),
        }],
        vec![raw_message(
            "gmail-1",
            "thread-1",
            "history-1",
            "resume-1@example.com",
        )],
    );
    let importer = FakeRfc822Importer::default();
    let mut options = GmailHistoricalImportOptions::into_mailboxes(["inbox"]);
    options.max_messages = Some(1);
    options.page_size = 1;

    let first = import_gmail_history(
        &pool,
        GmailHistoricalImportAccount {
            provider_account_id,
            user_id,
        },
        &first_gmail,
        &importer,
        options.clone(),
        &CancellationToken::new(),
    )
    .await
    .expect("first bounded import");
    assert!(!first.completed);
    assert_eq!(first.next_page_token.as_deref(), Some("page-2"));

    let stored_cursor: String =
        sqlx::query_scalar("SELECT backfill_cursor_json FROM provider_accounts WHERE id = ?1")
            .bind(provider_account_id)
            .fetch_one(&pool)
            .await
            .expect("stored cursor");
    assert!(stored_cursor.contains("page-2"));

    let second_gmail = FakeGmail::new(
        vec![ListMessagesResponse {
            messages: vec![ListMessage {
                id: "gmail-2".to_owned(),
                thread_id: Some("thread-2".to_owned()),
            }],
            next_page_token: None,
            result_size_estimate: Some(1),
        }],
        vec![raw_message(
            "gmail-2",
            "thread-2",
            "history-2",
            "resume-2@example.com",
        )],
    );
    let second = import_gmail_history(
        &pool,
        GmailHistoricalImportAccount {
            provider_account_id,
            user_id,
        },
        &second_gmail,
        &importer,
        options,
        &CancellationToken::new(),
    )
    .await
    .expect("second bounded import");

    assert_eq!(second.listed, 1);
    assert!(second.completed);
    assert_eq!(
        second_gmail.list_params()[0].page_token.as_deref(),
        Some("page-2")
    );
    let second_mapping = get_provider_message_mapping(&pool, provider_account_id, "gmail-2")
        .await
        .expect("mapping lookup")
        .expect("second mapping");
    assert_eq!(second_mapping.import_status, ProviderImportStatus::Imported);
}

#[tokio::test]
async fn full_rerun_after_completed_import_does_not_import_or_fetch_again() {
    let (pool, _guard, user_id, provider_account_id) = setup().await;
    let first_gmail = FakeGmail::new(
        vec![ListMessagesResponse {
            messages: vec![ListMessage {
                id: "gmail-idempotent".to_owned(),
                thread_id: Some("thread-idempotent".to_owned()),
            }],
            next_page_token: None,
            result_size_estimate: Some(1),
        }],
        vec![raw_message(
            "gmail-idempotent",
            "thread-idempotent",
            "history-idempotent",
            "idempotent@example.com",
        )],
    );
    let importer = FakeRfc822Importer::default();
    let mut options = GmailHistoricalImportOptions::into_mailboxes(["inbox"]);
    options.resume = false;

    let first = import_gmail_history(
        &pool,
        GmailHistoricalImportAccount {
            provider_account_id,
            user_id,
        },
        &first_gmail,
        &importer,
        options.clone(),
        &CancellationToken::new(),
    )
    .await
    .expect("first historical import");
    assert_eq!(first.imported, 1);
    assert_eq!(importer.imports().len(), 1);

    let second_gmail = FakeGmail::new(
        vec![ListMessagesResponse {
            messages: vec![ListMessage {
                id: "gmail-idempotent".to_owned(),
                thread_id: Some("thread-idempotent".to_owned()),
            }],
            next_page_token: None,
            result_size_estimate: Some(1),
        }],
        Vec::new(),
    );
    let second = import_gmail_history(
        &pool,
        GmailHistoricalImportAccount {
            provider_account_id,
            user_id,
        },
        &second_gmail,
        &importer,
        options,
        &CancellationToken::new(),
    )
    .await
    .expect("rerun historical import");

    assert_eq!(second.listed, 1);
    assert_eq!(second.skipped, 1);
    assert_eq!(second.imported, 0);
    assert_eq!(second.fetched, 0);
    assert!(second_gmail.raw_gets().is_empty());
    assert_eq!(importer.imports().len(), 1);
}

#[tokio::test]
async fn retries_failed_import_mapping_without_duplicating_stalwart_mail() {
    let (pool, _guard, user_id, provider_account_id) = setup().await;
    let importer = FakeRfc822Importer::default();
    importer.fail_next_for_provider_message_id(
        "gmail-retry",
        Rfc822ImportError::Jmap("transient failure".to_owned()),
    );
    let options = GmailHistoricalImportOptions::into_mailboxes(["inbox"]);

    let first_gmail = FakeGmail::new(
        vec![ListMessagesResponse {
            messages: vec![ListMessage {
                id: "gmail-retry".to_owned(),
                thread_id: Some("thread-retry".to_owned()),
            }],
            next_page_token: None,
            result_size_estimate: Some(1),
        }],
        vec![raw_message(
            "gmail-retry",
            "thread-retry",
            "history-retry-1",
            "retry@example.com",
        )],
    );
    let first = import_gmail_history(
        &pool,
        GmailHistoricalImportAccount {
            provider_account_id,
            user_id,
        },
        &first_gmail,
        &importer,
        options.clone(),
        &CancellationToken::new(),
    )
    .await
    .expect("first failed import pass");
    assert_eq!(first.failed, 1);
    let failed = get_provider_message_mapping(&pool, provider_account_id, "gmail-retry")
        .await
        .expect("failed mapping lookup")
        .expect("failed mapping");
    assert_eq!(failed.import_status, ProviderImportStatus::Failed);
    assert_eq!(
        failed.content_sha256.as_deref().map(|bytes| bytes.len()),
        Some(32)
    );

    let second_gmail = FakeGmail::new(
        vec![ListMessagesResponse {
            messages: vec![ListMessage {
                id: "gmail-retry".to_owned(),
                thread_id: Some("thread-retry".to_owned()),
            }],
            next_page_token: None,
            result_size_estimate: Some(1),
        }],
        vec![raw_message(
            "gmail-retry",
            "thread-retry",
            "history-retry-2",
            "retry@example.com",
        )],
    );
    let second = import_gmail_history(
        &pool,
        GmailHistoricalImportAccount {
            provider_account_id,
            user_id,
        },
        &second_gmail,
        &importer,
        options,
        &CancellationToken::new(),
    )
    .await
    .expect("retry import pass");

    assert_eq!(second.imported, 1);
    assert_eq!(second.failed, 0);
    assert_eq!(importer.imports().len(), 1);
    let imported = get_provider_message_mapping(&pool, provider_account_id, "gmail-retry")
        .await
        .expect("imported mapping lookup")
        .expect("imported mapping");
    assert_eq!(imported.import_status, ProviderImportStatus::Imported);
    assert_eq!(imported.jmap_email_id.as_deref(), Some("email-1"));
    assert_eq!(
        imported.provider_history_id.as_deref(),
        Some("history-retry-2")
    );
}
