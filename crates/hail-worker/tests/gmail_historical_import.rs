use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use hail_db::labels::{delete_label, list_labels, list_thread_labels, upsert_gmail_label};
use hail_db::provider_message_mappings::{
    ImportedProviderMessageMapping, ProviderImportStatus, get_provider_message_mapping,
    mark_provider_message_imported,
};
use hail_db::provider_sync_audit::list_provider_sync_audit_logs;
use hail_test::gmail_import_fixtures::{
    GmailImportFixture, GmailImportScenario, gmail_import_fixture,
};
use hail_worker::gmail_client::{
    GmailClientError, GmailLabel, ListMessage, ListMessagesParams, ListMessagesResponse,
    RawGmailMessage,
};
use hail_worker::gmail_historical_import::{
    GmailHistoricalImportAccount, GmailHistoricalImportError, GmailHistoricalImportOptions,
    GmailHistoricalImporter, GmailHistoricalSource, import_gmail_history,
};
use hail_worker::provider_import_routing::{
    RoutedImportedRfc822Message, RoutingRfc822Importer, ScreenerRfc822ImportRouter,
};
use hail_worker::rfc822_import::{
    FakeRfc822Importer, ImportedRfc822Message, Rfc822ImportError, Rfc822ImportRequest,
    Rfc822Importer,
};
use hail_worker::screener::{JmapOps, RouteError};
use tokio::sync::Barrier;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, PartialEq, Eq)]
enum JmapCall {
    GetOrCreateMailbox(String),
    GetMailboxByRole(String),
    ApplyKeyword {
        email_id: String,
        keyword: String,
    },
    RemoveKeyword {
        email_id: String,
        keyword: String,
    },
    MoveToMailbox {
        email_id: String,
        mailbox_id: String,
    },
}

#[derive(Debug)]
struct FakeJmapOps {
    calls: Mutex<Vec<JmapCall>>,
}

impl Default for FakeJmapOps {
    fn default() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
        }
    }
}

impl FakeJmapOps {
    fn calls(&self) -> Vec<JmapCall> {
        self.calls.lock().expect("jmap calls").clone()
    }
}

#[async_trait]
impl JmapOps for FakeJmapOps {
    async fn get_or_create_mailbox(&self, name: &str) -> Result<String, RouteError> {
        self.calls
            .lock()
            .expect("jmap calls")
            .push(JmapCall::GetOrCreateMailbox(name.to_string()));
        Ok(match name {
            hail_jmap::SCREENER_MAILBOX_NAME => "screener-id".to_string(),
            "Junk" => "junk-id".to_string(),
            other => format!("{}-id", other.to_ascii_lowercase()),
        })
    }

    async fn get_mailbox_by_role(&self, role: &str) -> Result<Option<String>, RouteError> {
        self.calls
            .lock()
            .expect("jmap calls")
            .push(JmapCall::GetMailboxByRole(role.to_string()));
        Ok(match role {
            "trash" => Some("trash-id".to_string()),
            "junk" => Some("junk-id".to_string()),
            _ => None,
        })
    }

    async fn apply_keyword(&self, email_id: &str, keyword: &str) -> Result<(), RouteError> {
        self.calls
            .lock()
            .expect("jmap calls")
            .push(JmapCall::ApplyKeyword {
                email_id: email_id.to_string(),
                keyword: keyword.to_string(),
            });
        Ok(())
    }

    async fn remove_keyword(&self, email_id: &str, keyword: &str) -> Result<(), RouteError> {
        self.calls
            .lock()
            .expect("jmap calls")
            .push(JmapCall::RemoveKeyword {
                email_id: email_id.to_string(),
                keyword: keyword.to_string(),
            });
        Ok(())
    }

    async fn move_to_mailbox(&self, email_id: &str, mailbox_id: &str) -> Result<(), RouteError> {
        self.calls
            .lock()
            .expect("jmap calls")
            .push(JmapCall::MoveToMailbox {
                email_id: email_id.to_string(),
                mailbox_id: mailbox_id.to_string(),
            });
        Ok(())
    }
}

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
          refresh_token_enc, sync_status, created_at, updated_at) \
         VALUES (?, ?, 'gmail', ?, ?, ?, 'active', ?, ?)",
    )
    .bind(user_id)
    .bind(jmap_account_id)
    .bind(format!("gmail-provider-{user_id}"))
    .bind(format!("user-{user_id}@gmail.example"))
    .bind(vec![1_u8; 29])
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
    labels: Arc<Vec<GmailLabel>>,
    list_params: Arc<Mutex<Vec<ListMessagesParams>>>,
    raw_gets: Arc<Mutex<Vec<String>>>,
}

impl FakeGmail {
    fn new(pages: Vec<ListMessagesResponse>, raw: Vec<RawGmailMessage>) -> Self {
        Self::with_labels(pages, raw, Vec::new())
    }

    fn with_labels(
        pages: Vec<ListMessagesResponse>,
        raw: Vec<RawGmailMessage>,
        labels: Vec<GmailLabel>,
    ) -> Self {
        Self {
            pages: Arc::new(Mutex::new(pages)),
            raw: Arc::new(
                raw.into_iter()
                    .map(|message| (message.id.clone(), message))
                    .collect(),
            ),
            labels: Arc::new(labels),
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

    async fn list_user_labels(&self) -> Result<Vec<GmailLabel>, GmailClientError> {
        Ok(self.labels.as_ref().clone())
    }
}

#[derive(Debug)]
struct BlockingHistoricalGmail {
    list_entered: Barrier,
    raw_entered: Barrier,
    block_list: bool,
}

impl BlockingHistoricalGmail {
    fn list_blocked() -> Self {
        Self {
            list_entered: Barrier::new(2),
            raw_entered: Barrier::new(1),
            block_list: true,
        }
    }

    fn raw_blocked() -> Self {
        Self {
            list_entered: Barrier::new(1),
            raw_entered: Barrier::new(2),
            block_list: false,
        }
    }
}

#[async_trait]
impl GmailHistoricalSource for BlockingHistoricalGmail {
    async fn list_messages(
        &self,
        _params: &ListMessagesParams,
    ) -> Result<ListMessagesResponse, GmailClientError> {
        self.list_entered.wait().await;
        if self.block_list {
            std::future::pending().await
        } else {
            Ok(ListMessagesResponse {
                messages: vec![ListMessage {
                    id: "gmail-block-raw".to_owned(),
                    thread_id: Some("thread-block".to_owned()),
                }],
                next_page_token: None,
                result_size_estimate: Some(1),
            })
        }
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

#[derive(Debug)]
struct BlockingHistoricalImporter {
    entered: Barrier,
}

#[async_trait]
impl GmailHistoricalImporter for BlockingHistoricalImporter {
    async fn import_gmail_rfc822(
        &self,
        _db: &sqlx::SqlitePool,
        _user_id: i64,
        _request: Rfc822ImportRequest,
    ) -> Result<RoutedImportedRfc822Message, GmailHistoricalImportError> {
        self.entered.wait().await;
        std::future::pending().await
    }
}

#[derive(Debug)]
struct BlockingRouter {
    entered: Barrier,
}

#[async_trait]
impl hail_worker::provider_import_routing::Rfc822ImportRouter for BlockingRouter {
    async fn route_imported_rfc822(
        &self,
        _conn: &mut sqlx::SqliteConnection,
        _user_id: i64,
        _imported: &ImportedRfc822Message,
        _request: &Rfc822ImportRequest,
    ) -> Result<hail_worker::screener::RouteOutcome, RouteError> {
        self.entered.wait().await;
        std::future::pending().await
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

fn list_fixture_page(
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
    list_thread_labels(pool, user_id, thread_id)
        .await
        .expect("thread labels")
        .into_iter()
        .map(|label| label.name)
        .collect()
}

#[tokio::test]
async fn imports_gmail_user_labels_to_local_thread_labels() {
    let (pool, _guard, user_id, provider_account_id) = setup().await;
    let gmail = FakeGmail::with_labels(
        vec![ListMessagesResponse {
            messages: vec![
                ListMessage {
                    id: "gmail-label-1".to_owned(),
                    thread_id: Some("gmail-thread-1".to_owned()),
                },
                ListMessage {
                    id: "gmail-label-2".to_owned(),
                    thread_id: Some("gmail-thread-1".to_owned()),
                },
            ],
            next_page_token: None,
            result_size_estimate: Some(2),
        }],
        vec![
            raw_message_with_labels(
                "gmail-label-1",
                "gmail-thread-1",
                "history-1",
                "label-1@example.com",
                &["Label_Work", "INBOX", "CATEGORY_PROMOTIONS", "Label_Nested"],
            ),
            raw_message_with_labels(
                "gmail-label-2",
                "gmail-thread-1",
                "history-2",
                "label-2@example.com",
                &["Label_Nested", "STARRED"],
            ),
        ],
        vec![
            gmail_label("Label_Work", "Work", "user"),
            gmail_label("Label_Nested", "Work/Receipts", "user"),
            gmail_label("INBOX", "Inbox", "system"),
            gmail_label("CATEGORY_PROMOTIONS", "Promotions", "system"),
            gmail_label("STARRED", "Starred", "system"),
        ],
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
    .expect("historical import");

    assert_eq!(summary.imported, 2);
    let labels = list_labels(&pool, user_id).await.expect("labels");
    assert_eq!(
        labels
            .iter()
            .map(|label| (label.name.as_str(), label.provider_label_id.as_deref()))
            .collect::<Vec<_>>(),
        vec![
            ("Work", Some("Label_Work")),
            ("Work/Receipts", Some("Label_Nested")),
        ]
    );
    assert_eq!(
        thread_label_names(&pool, user_id, "thread-1").await,
        vec!["Work".to_owned(), "Work/Receipts".to_owned()]
    );
    assert_eq!(
        thread_label_names(&pool, user_id, "thread-2").await,
        vec!["Work/Receipts".to_owned()]
    );
}

#[tokio::test]
async fn gmail_label_import_merges_manual_labels_and_recreates_deleted_labels() {
    let (pool, _guard, user_id, provider_account_id) = setup().await;
    let manual = hail_db::labels::create_label(&pool, user_id, " work / receipts ", None)
        .await
        .expect("manual label");
    let importer = FakeRfc822Importer::default();
    let options = GmailHistoricalImportOptions::into_mailboxes(["inbox"]);

    let first_gmail = FakeGmail::with_labels(
        vec![ListMessagesResponse {
            messages: vec![ListMessage {
                id: "gmail-merge-label".to_owned(),
                thread_id: Some("gmail-thread-merge".to_owned()),
            }],
            next_page_token: None,
            result_size_estimate: Some(1),
        }],
        vec![raw_message_with_labels(
            "gmail-merge-label",
            "gmail-thread-merge",
            "history-merge",
            "merge-label@example.com",
            &["Label_Receipts"],
        )],
        vec![gmail_label("Label_Receipts", "Work/Receipts", "user")],
    );

    import_gmail_history(
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
    .expect("first import");

    let labels = list_labels(&pool, user_id).await.expect("labels");
    assert_eq!(labels.len(), 1);
    assert_eq!(labels[0].id, manual.id);
    assert_eq!(
        labels[0].provider_label_id.as_deref(),
        Some("Label_Receipts")
    );

    delete_label(&pool, user_id, manual.id)
        .await
        .expect("delete label");
    let second_gmail = FakeGmail::with_labels(
        vec![ListMessagesResponse {
            messages: vec![ListMessage {
                id: "gmail-recreate-label".to_owned(),
                thread_id: Some("gmail-thread-recreate".to_owned()),
            }],
            next_page_token: None,
            result_size_estimate: Some(1),
        }],
        vec![raw_message_with_labels(
            "gmail-recreate-label",
            "gmail-thread-recreate",
            "history-recreate",
            "recreate-label@example.com",
            &["Label_Receipts"],
        )],
        vec![gmail_label("Label_Receipts", "Work/Receipts", "user")],
    );

    import_gmail_history(
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
    .expect("second import");

    let labels = list_labels(&pool, user_id).await.expect("labels");
    assert_eq!(labels.len(), 1);
    assert_eq!(labels[0].name, "Work/Receipts");
    assert_eq!(
        labels[0].provider_label_id.as_deref(),
        Some("Label_Receipts")
    );
    assert_eq!(
        thread_label_names(&pool, user_id, "thread-2").await,
        vec!["Work/Receipts".to_owned()]
    );
}

#[tokio::test]
async fn gmail_label_import_is_scoped_per_user() {
    let (pool, _guard, user_id, provider_account_id) = setup().await;
    let other_user = insert_user(&pool, "other-labels@example.com", "acct-other-labels").await;
    upsert_gmail_label(&pool, other_user, "Label_Shared", "Other/Only", None)
        .await
        .expect("other label");
    let gmail = FakeGmail::with_labels(
        vec![ListMessagesResponse {
            messages: vec![ListMessage {
                id: "gmail-scoped-label".to_owned(),
                thread_id: Some("gmail-thread-scoped".to_owned()),
            }],
            next_page_token: None,
            result_size_estimate: Some(1),
        }],
        vec![raw_message_with_labels(
            "gmail-scoped-label",
            "gmail-thread-scoped",
            "history-scoped",
            "scoped-label@example.com",
            &["Label_Shared"],
        )],
        vec![gmail_label("Label_Shared", "Mine/Only", "user")],
    );
    let importer = FakeRfc822Importer::default();

    import_gmail_history(
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
    .expect("import");

    assert_eq!(
        list_labels(&pool, user_id)
            .await
            .expect("user labels")
            .into_iter()
            .map(|label| label.name)
            .collect::<Vec<_>>(),
        vec!["Mine/Only".to_owned()]
    );
    assert_eq!(
        list_labels(&pool, other_user)
            .await
            .expect("other labels")
            .into_iter()
            .map(|label| label.name)
            .collect::<Vec<_>>(),
        vec!["Other/Only".to_owned()]
    );
}

fn hostile_leak_error() -> String {
    "provider failure Authorization: Bearer ya29.access-secret access_token=ya29.access-secret refresh_token=1//refresh-secret\r\n\r\nSubject: Private Body\r\n\r\nThis raw RFC822 body must not be stored".to_owned()
}

fn assert_no_hostile_leak(surface: &str) {
    for forbidden in [
        "Bearer",
        "ya29.access-secret",
        "1//refresh-secret",
        "Subject: Private Body",
        "This raw RFC822 body must not be stored",
    ] {
        assert!(
            !surface.contains(forbidden),
            "surface leaked {forbidden:?}: {surface}"
        );
    }
}

#[tokio::test]
async fn cancellation_interrupts_blocked_historical_list_fetch() {
    let (pool, _guard, user_id, provider_account_id) = setup().await;
    let gmail = Arc::new(BlockingHistoricalGmail::list_blocked());
    let importer = Arc::new(FakeRfc822Importer::default());
    let cancel = CancellationToken::new();
    let task_pool = pool.clone();
    let task_gmail = gmail.clone();
    let task_importer = importer.clone();
    let task_cancel = cancel.clone();
    let handle = tokio::spawn(async move {
        import_gmail_history(
            &task_pool,
            GmailHistoricalImportAccount {
                provider_account_id,
                user_id,
            },
            task_gmail.as_ref(),
            task_importer.as_ref(),
            GmailHistoricalImportOptions::into_mailboxes(["inbox"]),
            &task_cancel,
        )
        .await
    });

    gmail.list_entered.wait().await;
    cancel.cancel();
    let err = tokio::time::timeout(std::time::Duration::from_millis(200), handle)
        .await
        .expect("historical import should stop promptly")
        .expect("join")
        .expect_err("cancelled import");
    assert!(matches!(err, GmailHistoricalImportError::Cancelled));
    assert_eq!(
        account_error_class(&pool, provider_account_id)
            .await
            .as_deref(),
        Some("cancelled")
    );
    assert!(importer.imports().is_empty());
}

#[tokio::test]
async fn cancellation_interrupts_blocked_historical_raw_fetch() {
    let (pool, _guard, user_id, provider_account_id) = setup().await;
    let gmail = Arc::new(BlockingHistoricalGmail::raw_blocked());
    let importer = Arc::new(FakeRfc822Importer::default());
    let cancel = CancellationToken::new();
    let task_pool = pool.clone();
    let task_gmail = gmail.clone();
    let task_importer = importer.clone();
    let task_cancel = cancel.clone();
    let handle = tokio::spawn(async move {
        let options = GmailHistoricalImportOptions::into_mailboxes(["inbox"]);
        import_gmail_history(
            &task_pool,
            GmailHistoricalImportAccount {
                provider_account_id,
                user_id,
            },
            task_gmail.as_ref(),
            task_importer.as_ref(),
            options,
            &task_cancel,
        )
        .await
    });

    gmail.raw_entered.wait().await;
    cancel.cancel();
    let err = tokio::time::timeout(std::time::Duration::from_millis(200), handle)
        .await
        .expect("historical import should stop promptly")
        .expect("join")
        .expect_err("cancelled import");
    assert!(matches!(err, GmailHistoricalImportError::Cancelled));
    assert!(importer.imports().is_empty());
    assert!(
        get_provider_message_mapping(&pool, provider_account_id, "gmail-block-raw")
            .await
            .expect("mapping lookup")
            .is_none()
    );
}

#[tokio::test]
async fn cancellation_interrupts_blocked_historical_stalwart_import() {
    let (pool, _guard, user_id, provider_account_id) = setup().await;
    let gmail = Arc::new(FakeGmail::new(
        vec![ListMessagesResponse {
            messages: vec![ListMessage {
                id: "gmail-block-import".to_owned(),
                thread_id: Some("thread-block".to_owned()),
            }],
            next_page_token: None,
            result_size_estimate: Some(1),
        }],
        vec![raw_message(
            "gmail-block-import",
            "thread-block",
            "history-block",
            "block-import@example.com",
        )],
    ));
    let importer = Arc::new(BlockingHistoricalImporter {
        entered: Barrier::new(2),
    });
    let cancel = CancellationToken::new();
    let task_pool = pool.clone();
    let task_gmail = gmail.clone();
    let task_importer = importer.clone();
    let task_cancel = cancel.clone();
    let handle = tokio::spawn(async move {
        import_gmail_history(
            &task_pool,
            GmailHistoricalImportAccount {
                provider_account_id,
                user_id,
            },
            task_gmail.as_ref(),
            task_importer.as_ref(),
            GmailHistoricalImportOptions::into_mailboxes(["inbox"]),
            &task_cancel,
        )
        .await
    });

    importer.entered.wait().await;
    cancel.cancel();
    let err = tokio::time::timeout(std::time::Duration::from_millis(200), handle)
        .await
        .expect("historical import should stop promptly")
        .expect("join")
        .expect_err("cancelled import");
    assert!(matches!(err, GmailHistoricalImportError::Cancelled));
    let mapping = get_provider_message_mapping(&pool, provider_account_id, "gmail-block-import")
        .await
        .expect("mapping lookup")
        .expect("seen mapping");
    assert_eq!(mapping.import_status, ProviderImportStatus::Pending);
    assert!(mapping.jmap_email_id.is_none());
}

#[tokio::test]
async fn cancellation_interrupts_blocked_historical_routing() {
    let (pool, _guard, user_id, provider_account_id) = setup().await;
    let gmail = Arc::new(FakeGmail::new(
        vec![ListMessagesResponse {
            messages: vec![ListMessage {
                id: "gmail-block-route".to_owned(),
                thread_id: Some("thread-block".to_owned()),
            }],
            next_page_token: None,
            result_size_estimate: Some(1),
        }],
        vec![raw_message(
            "gmail-block-route",
            "thread-block",
            "history-block",
            "block-route@example.com",
        )],
    ));
    let importer = Arc::new(FakeRfc822Importer::default());
    let router = Arc::new(BlockingRouter {
        entered: Barrier::new(2),
    });
    let cancel = CancellationToken::new();
    let task_pool = pool.clone();
    let task_gmail = gmail.clone();
    let task_importer = importer.clone();
    let task_router = router.clone();
    let task_cancel = cancel.clone();
    let handle = tokio::spawn(async move {
        let routed = RoutingRfc822Importer::new(task_importer.as_ref(), task_router.as_ref());
        import_gmail_history(
            &task_pool,
            GmailHistoricalImportAccount {
                provider_account_id,
                user_id,
            },
            task_gmail.as_ref(),
            &routed,
            GmailHistoricalImportOptions::into_mailboxes(["inbox"]),
            &task_cancel,
        )
        .await
    });

    router.entered.wait().await;
    cancel.cancel();
    let err = tokio::time::timeout(std::time::Duration::from_millis(200), handle)
        .await
        .expect("historical import should stop promptly")
        .expect("join")
        .expect_err("cancelled import");
    assert!(matches!(err, GmailHistoricalImportError::Cancelled));
    assert_eq!(importer.imports().len(), 1);
    let mapping = get_provider_message_mapping(&pool, provider_account_id, "gmail-block-route")
        .await
        .expect("mapping lookup")
        .expect("seen mapping");
    assert_eq!(mapping.import_status, ProviderImportStatus::Imported);
    assert_eq!(mapping.jmap_email_id.as_deref(), Some("email-1"));
}

async fn account_error_class(pool: &sqlx::SqlitePool, provider_account_id: i64) -> Option<String> {
    sqlx::query_scalar("SELECT last_error_class FROM provider_accounts WHERE id = ?1")
        .bind(provider_account_id)
        .fetch_one(pool)
        .await
        .expect("account error class")
}

#[tokio::test]
async fn imports_gmail_pages_into_stalwart_and_records_mapping_and_audit() {
    let (pool, _guard, user_id, provider_account_id) = setup().await;
    let first_fixture = gmail_import_fixture(GmailImportScenario::RawRfc822Import);
    let second_fixture = gmail_import_fixture(GmailImportScenario::RoutingFeed);
    let gmail = FakeGmail::new(
        vec![list_fixture_page([first_fixture, second_fixture])],
        vec![
            raw_fixture_message(first_fixture),
            raw_fixture_message(second_fixture),
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

    let first = get_provider_message_mapping(&pool, provider_account_id, first_fixture.gmail_id)
        .await
        .expect("mapping lookup")
        .expect("mapping exists");
    assert_eq!(first.import_status, ProviderImportStatus::Imported);
    assert_eq!(
        first.provider_thread_id.as_deref(),
        Some(first_fixture.thread_id)
    );
    assert_eq!(
        first.provider_history_id.as_deref(),
        Some(first_fixture.history_id)
    );
    assert_eq!(
        first.rfc822_message_id.as_deref(),
        Some(first_fixture.rfc822_message_id)
    );
    assert_eq!(first.jmap_email_id.as_deref(), Some("email-1"));

    let imports = importer.imports();
    assert_eq!(imports.len(), 2);
    assert_eq!(imports[0].mailbox_ids, vec!["inbox"]);
    assert_eq!(imports[0].keywords, vec!["$seen"]);
    assert_eq!(imports[0].raw_rfc822, first_fixture.raw_rfc822());
    assert_eq!(
        gmail.raw_gets(),
        vec![first_fixture.gmail_id, second_fixture.gmail_id]
    );
    assert_eq!(gmail.list_params()[0].label_ids, vec!["INBOX"]);
    assert_eq!(
        gmail.list_params()[0].query.as_deref(),
        Some("newer_than:30d -in:sent")
    );
    assert!(!gmail.list_params()[0].include_spam_trash);

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
async fn defaults_exclude_spam_trash_and_sent_from_inbound_import() {
    let (pool, _guard, user_id, provider_account_id) = setup().await;
    let gmail = FakeGmail::new(
        vec![ListMessagesResponse {
            messages: Vec::new(),
            next_page_token: None,
            result_size_estimate: Some(0),
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
    .expect("empty historical import");

    assert!(summary.completed);
    let params = gmail.list_params();
    assert_eq!(params.len(), 1);
    assert_eq!(params[0].query.as_deref(), Some("-in:sent"));
    assert!(params[0].label_ids.is_empty());
    assert!(!params[0].include_spam_trash);
    assert!(importer.imports().is_empty());
}

#[tokio::test]
async fn provider_labels_are_only_import_hints_not_local_keywords() {
    let (pool, _guard, user_id, provider_account_id) = setup().await;
    let gmail = FakeGmail::new(
        vec![ListMessagesResponse {
            messages: vec![ListMessage {
                id: "gmail-hinted".to_owned(),
                thread_id: Some("thread-hinted".to_owned()),
            }],
            next_page_token: None,
            result_size_estimate: Some(1),
        }],
        vec![raw_message(
            "gmail-hinted",
            "thread-hinted",
            "history-hinted",
            "hinted@example.com",
        )],
    );
    let importer = FakeRfc822Importer::default();
    let mut options = GmailHistoricalImportOptions::into_mailboxes(["inbox"]);
    options.label_ids = vec!["STARRED".to_owned(), "CATEGORY_PROMOTIONS".to_owned()];
    options.keywords = Vec::new();

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
    .expect("hinted historical import");

    assert_eq!(summary.imported, 1);
    assert_eq!(
        gmail.list_params()[0].label_ids,
        ["STARRED", "CATEGORY_PROMOTIONS"]
    );
    let imports = importer.imports();
    assert_eq!(imports.len(), 1);
    assert!(imports[0].keywords.is_empty());
    assert_eq!(imports[0].mailbox_ids, vec!["inbox"]);
}

#[tokio::test]
async fn explicit_sent_copy_import_can_disable_default_sent_exclusion() {
    let (pool, _guard, user_id, provider_account_id) = setup().await;
    let sent_copy = gmail_import_fixture(GmailImportScenario::SentCopyOneWay);
    let gmail = FakeGmail::new(
        vec![list_fixture_page([sent_copy])],
        vec![raw_fixture_message(sent_copy)],
    );
    let importer = FakeRfc822Importer::default();
    let mut options = GmailHistoricalImportOptions::into_mailboxes(["sent"]);
    options.exclude_sent = false;
    options.label_ids = vec!["SENT".to_owned()];

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
    .expect("explicit sent import window");

    assert_eq!(summary.imported, 1);
    assert_eq!(gmail.list_params()[0].label_ids, ["SENT"]);
    assert_eq!(gmail.list_params()[0].query, None);
    let imports = importer.imports();
    assert_eq!(imports.len(), 1);
    assert_eq!(imports[0].mailbox_ids, vec!["sent"]);
    assert!(imports[0].keywords.is_empty());
    assert_eq!(imports[0].raw_rfc822, sent_copy.raw_rfc822());

    let mapping = get_provider_message_mapping(&pool, provider_account_id, sent_copy.gmail_id)
        .await
        .expect("mapping lookup")
        .expect("sent copy mapping");
    assert_eq!(mapping.import_status, ProviderImportStatus::Imported);
    assert_eq!(
        mapping.rfc822_message_id.as_deref(),
        Some(sent_copy.rfc822_message_id)
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
    let original = gmail_import_fixture(GmailImportScenario::RawRfc822Import);
    let copy_fixture = gmail_import_fixture(GmailImportScenario::DedupeIdempotency);
    mark_provider_message_imported(
        &pool,
        ImportedProviderMessageMapping {
            provider_account_id,
            provider_message_id: original.gmail_id,
            provider_thread_id: Some(original.thread_id),
            provider_history_id: Some(original.history_id),
            rfc822_message_id: Some(original.rfc822_message_id),
            content_sha256: None,
            jmap_email_id: "local-original",
            jmap_thread_id: Some("local-thread"),
            jmap_mailbox_ids_json: Some(r#"["inbox"]"#),
        },
    )
    .await
    .expect("seed mapping");
    let gmail = FakeGmail::new(
        vec![list_fixture_page([copy_fixture])],
        vec![raw_fixture_message(copy_fixture)],
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
    let copy = get_provider_message_mapping(&pool, provider_account_id, copy_fixture.gmail_id)
        .await
        .expect("mapping lookup")
        .expect("copy mapping");
    assert_eq!(copy.import_status, ProviderImportStatus::Duplicate);
    assert_eq!(copy.jmap_email_id.as_deref(), Some("local-original"));
    assert_eq!(
        copy.rfc822_message_id.as_deref(),
        Some(copy_fixture.rfc822_message_id)
    );
}

#[tokio::test]
async fn configured_bound_limits_list_fetch_import_and_audits_explicitly() {
    let (pool, _guard, user_id, provider_account_id) = setup().await;
    let gmail = FakeGmail::new(
        vec![ListMessagesResponse {
            messages: vec![
                ListMessage {
                    id: "gmail-bound-1".to_owned(),
                    thread_id: Some("thread-bound-1".to_owned()),
                },
                ListMessage {
                    id: "gmail-bound-2".to_owned(),
                    thread_id: Some("thread-bound-2".to_owned()),
                },
                ListMessage {
                    id: "gmail-bound-3".to_owned(),
                    thread_id: Some("thread-bound-3".to_owned()),
                },
            ],
            next_page_token: Some("page-after-bound".to_owned()),
            result_size_estimate: Some(99),
        }],
        vec![
            raw_message(
                "gmail-bound-1",
                "thread-bound-1",
                "history-bound-1",
                "bound-1@example.com",
            ),
            raw_message(
                "gmail-bound-2",
                "thread-bound-2",
                "history-bound-2",
                "bound-2@example.com",
            ),
            raw_message(
                "gmail-bound-3",
                "thread-bound-3",
                "history-bound-3",
                "bound-3@example.com",
            ),
        ],
    );
    let importer = FakeRfc822Importer::default();
    let mut options = GmailHistoricalImportOptions::into_mailboxes(["inbox"]);
    options.max_messages = Some(2);
    options.page_size = 50;

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
    .expect("bounded historical import");

    assert_eq!(summary.listed, 2);
    assert_eq!(summary.fetched, 2);
    assert_eq!(summary.imported, 2);
    assert_eq!(summary.pages, 1);
    assert!(!summary.completed);
    assert!(summary.bounded);
    assert_eq!(summary.bound_max_messages, Some(2));
    assert_eq!(summary.next_page_token.as_deref(), Some("page-after-bound"));
    assert_eq!(gmail.list_params()[0].max_results, Some(2));
    assert_eq!(
        gmail.raw_gets(),
        vec!["gmail-bound-1".to_owned(), "gmail-bound-2".to_owned()]
    );
    assert_eq!(importer.imports().len(), 2);
    assert!(
        get_provider_message_mapping(&pool, provider_account_id, "gmail-bound-3")
            .await
            .expect("mapping lookup")
            .is_none(),
        "messages beyond the configured bound must not be fetched/imported"
    );

    let (status, completed_at, cursor_json): (String, Option<String>, String) = sqlx::query_as(
        "SELECT sync_status, initial_sync_completed_at, backfill_cursor_json \
         FROM provider_accounts WHERE id = ?1",
    )
    .bind(provider_account_id)
    .fetch_one(&pool)
    .await
    .expect("provider account state");
    assert_eq!(status, "initial_sync");
    assert!(completed_at.is_none());
    assert!(cursor_json.contains("page-after-bound"));
    assert!(cursor_json.contains("\"max_messages\""));

    let audit = list_provider_sync_audit_logs(&pool, user_id, provider_account_id, 20)
        .await
        .expect("audit logs");
    assert!(audit.iter().any(|log| {
        log.event_type == "message_skipped"
            && log.safe_error_class.as_deref() == Some("configured_initial_import_bound")
    }));
    assert!(audit.iter().any(|log| {
        log.event_type == "sync_completed"
            && log
                .metadata_json
                .as_deref()
                .is_some_and(|json| json.contains("\"bounded\":true"))
    }));
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
        options.clone(),
        &CancellationToken::new(),
    )
    .await
    .expect("same bounded import remains capped");

    assert_eq!(second.listed, 0);
    assert!(!second.completed);
    assert!(second.bounded);
    assert!(second_gmail.list_params().is_empty());
    assert!(
        get_provider_message_mapping(&pool, provider_account_id, "gmail-2")
            .await
            .expect("mapping lookup")
            .is_none()
    );

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
    options.max_messages = Some(2);
    let third = import_gmail_history(
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
    .expect("raised bound import");

    assert_eq!(third.listed, 1);
    assert!(third.completed);
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
async fn corrupt_resume_cursor_surfaces_error_and_audit() {
    let (pool, _guard, user_id, provider_account_id) = setup().await;
    sqlx::query("DROP TRIGGER provider_accounts_json_state_update")
        .execute(&pool)
        .await
        .expect("drop json state update trigger for corruption simulation");
    sqlx::query("UPDATE provider_accounts SET backfill_cursor_json = ?1 WHERE id = ?2")
        .bind("{not-json")
        .bind(provider_account_id)
        .execute(&pool)
        .await
        .expect("simulate durable cursor corruption");

    let gmail = FakeGmail::new(
        vec![ListMessagesResponse {
            messages: vec![ListMessage {
                id: "gmail-corrupt-cursor".to_owned(),
                thread_id: Some("thread-corrupt-cursor".to_owned()),
            }],
            next_page_token: None,
            result_size_estimate: Some(1),
        }],
        vec![raw_message(
            "gmail-corrupt-cursor",
            "thread-corrupt-cursor",
            "history-corrupt-cursor",
            "corrupt-cursor@example.com",
        )],
    );
    let importer = FakeRfc822Importer::default();
    let options = GmailHistoricalImportOptions::into_mailboxes(["inbox"]);

    let result = import_gmail_history(
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
    .await;

    assert!(
        result.is_err(),
        "corrupt durable cursor must not silently restart import"
    );
    assert!(
        gmail.list_params().is_empty(),
        "import must fail before issuing Gmail list with reset cursor semantics"
    );
    let (sync_status, last_error_class): (String, Option<String>) =
        sqlx::query_as("SELECT sync_status, last_error_class FROM provider_accounts WHERE id = ?1")
            .bind(provider_account_id)
            .fetch_one(&pool)
            .await
            .expect("account status");
    assert_eq!(sync_status, "error");
    assert_eq!(last_error_class.as_deref(), Some("provider_cursor_corrupt"));
    let logs = list_provider_sync_audit_logs(&pool, user_id, provider_account_id, 10)
        .await
        .expect("audit logs");
    assert!(logs.iter().any(|log| log.event_type == "sync_failed"
        && log.safe_error_class.as_deref() == Some("provider_cursor_corrupt")));
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
async fn failed_message_import_redacts_tokens_and_raw_body_from_mapping_and_audit() {
    let (pool, _guard, user_id, provider_account_id) = setup().await;
    let importer = FakeRfc822Importer::default();
    importer.fail_next_for_provider_message_id(
        "gmail-hostile-leak",
        Rfc822ImportError::Jmap(hostile_leak_error()),
    );
    let gmail = FakeGmail::new(
        vec![ListMessagesResponse {
            messages: vec![ListMessage {
                id: "gmail-hostile-leak".to_owned(),
                thread_id: Some("thread-hostile".to_owned()),
            }],
            next_page_token: None,
            result_size_estimate: Some(1),
        }],
        vec![RawGmailMessage {
            id: "gmail-hostile-leak".to_owned(),
            thread_id: Some("thread-hostile".to_owned()),
            history_id: Some("history-hostile".to_owned()),
            label_ids: Vec::new(),
            rfc822: b"From: sender@example.com\r\nTo: user@example.com\r\nMessage-ID: <hostile@example.com>\r\nSubject: imported body\r\n\r\nImported raw body is not an error".to_vec(),
        }],
    );

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
    .expect("historical import records failed message");

    assert_eq!(summary.failed, 1);
    let mapping = get_provider_message_mapping(&pool, provider_account_id, "gmail-hostile-leak")
        .await
        .expect("mapping lookup")
        .expect("failed mapping");
    assert_eq!(mapping.import_status, ProviderImportStatus::Failed);
    let mapping_error = mapping.error_message.expect("mapping error message");
    assert_no_hostile_leak(&mapping_error);
    assert!(mapping_error.contains("[redacted]"));

    let audit = list_provider_sync_audit_logs(&pool, user_id, provider_account_id, 10)
        .await
        .expect("audit logs");
    let failed_event = audit
        .iter()
        .find(|event| event.event_type == "message_failed")
        .expect("message_failed audit event");
    let audit_error = failed_event
        .safe_error_message
        .as_deref()
        .expect("audit safe error message");
    assert_no_hostile_leak(audit_error);
    assert!(audit_error.contains("[redacted]"));
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
    ) -> Result<RoutedImportedRfc822Message, GmailHistoricalImportError> {
        let imported = self.inner.import_rfc822(request.clone()).await?;
        let mut fail_next = self
            .fail_next_for_provider_id
            .lock()
            .expect("post import crash flag");
        if fail_next.as_deref() == request.provider_message_id.as_deref() {
            *fail_next = None;
            return Err(GmailHistoricalImportError::Rfc822Import(
                Rfc822ImportError::Jmap("simulated crash after local import".to_owned()),
            ));
        }
        Ok(RoutedImportedRfc822Message {
            imported,
            route_outcome: None,
        })
    }
}

async fn backfill_cursor_json(
    pool: &sqlx::SqlitePool,
    provider_account_id: i64,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT backfill_cursor_json FROM provider_accounts WHERE id = ?1")
        .bind(provider_account_id)
        .fetch_one(pool)
        .await
}

async fn audit_event_count(
    pool: &sqlx::SqlitePool,
    provider_account_id: i64,
    event_type: &str,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM provider_sync_events WHERE provider_account_id = ?1 AND event_type = ?2",
    )
    .bind(provider_account_id)
    .bind(event_type)
    .fetch_one(pool)
    .await
}

#[tokio::test]
async fn crash_after_stalwart_import_before_mapping_retries_without_duplicate_local_mail() {
    let (pool, _guard, user_id, provider_account_id) = setup().await;
    let importer = FakeRfc822Importer::default();
    let crashing_importer = PostImportCrashImporter::new(&importer, "gmail-post-import-crash");
    let options = GmailHistoricalImportOptions::into_mailboxes(["inbox"]);

    let first_gmail = FakeGmail::new(
        vec![ListMessagesResponse {
            messages: vec![ListMessage {
                id: "gmail-post-import-crash".to_owned(),
                thread_id: Some("thread-post-import-crash".to_owned()),
            }],
            next_page_token: None,
            result_size_estimate: Some(1),
        }],
        vec![raw_message(
            "gmail-post-import-crash",
            "thread-post-import-crash",
            "history-post-import-crash-1",
            "post-import-crash@example.com",
        )],
    );
    let first = import_gmail_history(
        &pool,
        GmailHistoricalImportAccount {
            provider_account_id,
            user_id,
        },
        &first_gmail,
        &crashing_importer,
        options.clone(),
        &CancellationToken::new(),
    )
    .await
    .expect("first pass records failure after local import");

    assert_eq!(first.failed, 1);
    assert_eq!(first.imported, 0);
    assert_eq!(importer.imports().len(), 1);
    assert_eq!(importer.local_message_count(), 1);
    let failed =
        get_provider_message_mapping(&pool, provider_account_id, "gmail-post-import-crash")
            .await
            .expect("failed mapping lookup")
            .expect("failed mapping");
    assert_eq!(failed.import_status, ProviderImportStatus::Failed);
    assert_eq!(failed.error_class.as_deref(), Some("stalwart_import"));
    assert!(failed.jmap_email_id.is_none());
    let cursor_after_failure = backfill_cursor_json(&pool, provider_account_id)
        .await
        .expect("cursor query");
    assert!(
        cursor_after_failure
            .as_deref()
            .is_some_and(|cursor| cursor.contains("\"completed\":true")),
        "the failed pass still persists page progress: {cursor_after_failure:?}"
    );

    let second_gmail = FakeGmail::new(
        vec![ListMessagesResponse {
            messages: vec![ListMessage {
                id: "gmail-post-import-crash".to_owned(),
                thread_id: Some("thread-post-import-crash".to_owned()),
            }],
            next_page_token: None,
            result_size_estimate: Some(1),
        }],
        vec![raw_message(
            "gmail-post-import-crash",
            "thread-post-import-crash",
            "history-post-import-crash-2",
            "post-import-crash@example.com",
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
    .expect("retry after crash window");

    assert_eq!(second.failed, 0);
    assert_eq!(second.imported, 0);
    assert_eq!(second.duplicates, 1);
    assert_eq!(importer.local_message_count(), 1);
    assert_eq!(
        importer.imports().len(),
        2,
        "retry may call import, but fake Stalwart must dedupe it"
    );
    let mapping =
        get_provider_message_mapping(&pool, provider_account_id, "gmail-post-import-crash")
            .await
            .expect("mapping lookup")
            .expect("mapping converged");
    assert_eq!(mapping.import_status, ProviderImportStatus::Duplicate);
    assert_eq!(mapping.jmap_email_id.as_deref(), Some("email-1"));
    assert_eq!(
        mapping.rfc822_message_id.as_deref(),
        Some("post-import-crash@example.com")
    );
    assert_eq!(
        mapping.provider_history_id.as_deref(),
        Some("history-post-import-crash-2")
    );
    assert!(mapping.error_class.is_none());

    let audit = list_provider_sync_audit_logs(&pool, user_id, provider_account_id, 20)
        .await
        .expect("audit logs");
    assert!(audit.iter().any(|log| {
        log.event_type == "message_failed"
            && log.provider_message_id.as_deref() == Some("gmail-post-import-crash")
            && log.safe_error_class.as_deref() == Some("stalwart_import")
    }));
    assert!(audit.iter().any(|log| {
        log.event_type == "message_imported"
            && log.provider_message_id.as_deref() == Some("gmail-post-import-crash")
            && log.result_status == "succeeded"
            && log
                .metadata_json
                .as_deref()
                .is_some_and(|metadata| metadata.contains("\"duplicate\":true"))
    }));
    assert_eq!(
        audit_event_count(&pool, provider_account_id, "message_imported")
            .await
            .expect("audit count"),
        1
    );
}

#[derive(Debug, Default)]
struct FailingThenOkJmapOps {
    calls: Mutex<Vec<JmapCall>>,
    fail_next_apply_keyword: Mutex<bool>,
}

impl FailingThenOkJmapOps {
    fn calls(&self) -> Vec<JmapCall> {
        self.calls.lock().expect("jmap calls").clone()
    }
}

#[async_trait]
impl JmapOps for FailingThenOkJmapOps {
    async fn get_or_create_mailbox(&self, name: &str) -> Result<String, RouteError> {
        self.calls
            .lock()
            .expect("jmap calls")
            .push(JmapCall::GetOrCreateMailbox(name.to_string()));
        Ok(format!("{}-id", name.to_ascii_lowercase()))
    }

    async fn get_mailbox_by_role(&self, role: &str) -> Result<Option<String>, RouteError> {
        self.calls
            .lock()
            .expect("jmap calls")
            .push(JmapCall::GetMailboxByRole(role.to_string()));
        Ok(Some(format!("{role}-id")))
    }

    async fn apply_keyword(&self, email_id: &str, keyword: &str) -> Result<(), RouteError> {
        self.calls
            .lock()
            .expect("jmap calls")
            .push(JmapCall::ApplyKeyword {
                email_id: email_id.to_string(),
                keyword: keyword.to_string(),
            });
        let mut fail_next = self
            .fail_next_apply_keyword
            .lock()
            .expect("fail_next_apply_keyword");
        if *fail_next {
            *fail_next = false;
            return Err(RouteError::Jmap("transient route failure".to_string()));
        }
        Ok(())
    }

    async fn remove_keyword(&self, email_id: &str, keyword: &str) -> Result<(), RouteError> {
        self.calls
            .lock()
            .expect("jmap calls")
            .push(JmapCall::RemoveKeyword {
                email_id: email_id.to_string(),
                keyword: keyword.to_string(),
            });
        Ok(())
    }

    async fn move_to_mailbox(&self, email_id: &str, mailbox_id: &str) -> Result<(), RouteError> {
        self.calls
            .lock()
            .expect("jmap calls")
            .push(JmapCall::MoveToMailbox {
                email_id: email_id.to_string(),
                mailbox_id: mailbox_id.to_string(),
            });
        Ok(())
    }
}

#[tokio::test]
async fn routed_import_failure_preserves_mapping_and_retries_route_without_duplicate_import() {
    let (pool, _guard, user_id, provider_account_id) = setup().await;
    sqlx::query(
        "INSERT INTO screener_rules \
         (user_id, sender_address, decision, classify_as, decided_at, first_seen_at) \
         VALUES (?, 'sender@example.com', 'allow', 'imbox', ?, ?)",
    )
    .bind(user_id)
    .bind("2026-01-01T00:00:00Z")
    .bind("2026-01-01T00:00:00Z")
    .execute(&pool)
    .await
    .expect("allow rule");

    let importer = FakeRfc822Importer::default();
    let jmap = FailingThenOkJmapOps {
        fail_next_apply_keyword: Mutex::new(true),
        ..FailingThenOkJmapOps::default()
    };
    let router = ScreenerRfc822ImportRouter::new(&jmap);
    let routed_importer = RoutingRfc822Importer::new(&importer, &router);
    let options = GmailHistoricalImportOptions::into_mailboxes(["inbox"]);

    let first_gmail = FakeGmail::new(
        vec![ListMessagesResponse {
            messages: vec![ListMessage {
                id: "gmail-route-retry".to_owned(),
                thread_id: Some("thread-route-retry".to_owned()),
            }],
            next_page_token: None,
            result_size_estimate: Some(1),
        }],
        vec![raw_message(
            "gmail-route-retry",
            "thread-route-retry",
            "history-route-retry-1",
            "route-retry@example.com",
        )],
    );
    let first = import_gmail_history(
        &pool,
        GmailHistoricalImportAccount {
            provider_account_id,
            user_id,
        },
        &first_gmail,
        &routed_importer,
        options.clone(),
        &CancellationToken::new(),
    )
    .await
    .expect("first routed import pass");

    assert_eq!(first.failed, 1);
    assert_eq!(first.imported, 0);
    assert_eq!(importer.imports().len(), 1);
    let failed_route =
        get_provider_message_mapping(&pool, provider_account_id, "gmail-route-retry")
            .await
            .expect("mapping lookup")
            .expect("mapping retained after route failure");
    assert_eq!(failed_route.import_status, ProviderImportStatus::Imported);
    assert_eq!(failed_route.jmap_email_id.as_deref(), Some("email-1"));
    assert_eq!(failed_route.error_class.as_deref(), Some("route_import"));

    let second_gmail = FakeGmail::new(
        vec![ListMessagesResponse {
            messages: vec![ListMessage {
                id: "gmail-route-retry".to_owned(),
                thread_id: Some("thread-route-retry".to_owned()),
            }],
            next_page_token: None,
            result_size_estimate: Some(1),
        }],
        vec![raw_message(
            "gmail-route-retry",
            "thread-route-retry",
            "history-route-retry-2",
            "route-retry@example.com",
        )],
    );
    let second = import_gmail_history(
        &pool,
        GmailHistoricalImportAccount {
            provider_account_id,
            user_id,
        },
        &second_gmail,
        &routed_importer,
        options,
        &CancellationToken::new(),
    )
    .await
    .expect("retry routed import pass");

    assert_eq!(second.failed, 0);
    assert_eq!(second.skipped, 1);
    assert_eq!(importer.imports().len(), 1);
    let imported = get_provider_message_mapping(&pool, provider_account_id, "gmail-route-retry")
        .await
        .expect("mapping lookup")
        .expect("imported mapping");
    assert_eq!(imported.import_status, ProviderImportStatus::Imported);
    assert_eq!(imported.jmap_email_id.as_deref(), Some("email-1"));
    assert!(imported.error_class.is_none());
    assert_eq!(
        jmap.calls(),
        vec![
            JmapCall::ApplyKeyword {
                email_id: "email-1".to_string(),
                keyword: "$hail_imbox".to_string(),
            },
            JmapCall::ApplyKeyword {
                email_id: "email-1".to_string(),
                keyword: "$hail_imbox".to_string(),
            },
        ]
    );
}

#[tokio::test]
async fn routed_import_applies_allowed_sender_classifications() {
    for scenario in [
        GmailImportScenario::RoutingImbox,
        GmailImportScenario::RoutingFeed,
        GmailImportScenario::RoutingPapertrail,
    ] {
        let (pool, _guard, user_id, provider_account_id) = setup().await;
        let fixture = gmail_import_fixture(scenario);
        let route = fixture.intended_route.expect("fixture has route");
        sqlx::query(
            "INSERT INTO screener_rules \
             (user_id, sender_address, decision, classify_as, decided_at, first_seen_at) \
             VALUES (?, ?, 'allow', ?, ?, ?)",
        )
        .bind(user_id)
        .bind(route.sender)
        .bind(route.classify_as.expect("allowed fixture classify_as"))
        .bind("2026-01-01T00:00:00Z")
        .bind("2026-01-01T00:00:00Z")
        .execute(&pool)
        .await
        .expect("allow rule");
        let gmail = FakeGmail::new(
            vec![list_fixture_page([fixture])],
            vec![raw_fixture_message(fixture)],
        );
        let importer = FakeRfc822Importer::default();
        let jmap = FakeJmapOps::default();
        let router = ScreenerRfc822ImportRouter::new(&jmap);
        let routed_importer = RoutingRfc822Importer::new(&importer, &router);

        let summary = import_gmail_history(
            &pool,
            GmailHistoricalImportAccount {
                provider_account_id,
                user_id,
            },
            &gmail,
            &routed_importer,
            GmailHistoricalImportOptions::into_mailboxes(["inbox"]),
            &CancellationToken::new(),
        )
        .await
        .expect("routed import");

        assert_eq!(summary.imported, 1);
        assert_eq!(
            jmap.calls(),
            vec![JmapCall::ApplyKeyword {
                email_id: "email-1".to_string(),
                keyword: route.keyword.expect("route keyword").to_string(),
            }]
        );
        let event_type: String = sqlx::query_scalar("SELECT event_type FROM app_events")
            .fetch_one(&pool)
            .await
            .expect("app event");
        assert_eq!(
            event_type,
            match scenario {
                GmailImportScenario::RoutingImbox => "imbox.new",
                GmailImportScenario::RoutingFeed => "feed.new",
                GmailImportScenario::RoutingPapertrail => "papertrail.new",
                _ => unreachable!("only classified routing scenarios are tested here"),
            }
        );
    }
}

#[tokio::test]
async fn routed_import_sends_unknown_sender_to_screener_pending() {
    let (pool, _guard, user_id, provider_account_id) = setup().await;
    let fixture = gmail_import_fixture(GmailImportScenario::RoutingScreener);
    let gmail = FakeGmail::new(
        vec![list_fixture_page([fixture])],
        vec![raw_fixture_message(fixture)],
    );
    let importer = FakeRfc822Importer::default();
    let jmap = FakeJmapOps::default();
    let router = ScreenerRfc822ImportRouter::new(&jmap);
    let routed_importer = RoutingRfc822Importer::new(&importer, &router);

    let summary = import_gmail_history(
        &pool,
        GmailHistoricalImportAccount {
            provider_account_id,
            user_id,
        },
        &gmail,
        &routed_importer,
        GmailHistoricalImportOptions::into_mailboxes(["inbox"]),
        &CancellationToken::new(),
    )
    .await
    .expect("routed import");

    assert_eq!(summary.imported, 1);
    assert_eq!(
        jmap.calls(),
        vec![
            JmapCall::GetOrCreateMailbox("Screener".to_string()),
            JmapCall::MoveToMailbox {
                email_id: "email-1".to_string(),
                mailbox_id: "screener-id".to_string(),
            },
            JmapCall::RemoveKeyword {
                email_id: "email-1".to_string(),
                keyword: "$hail_imbox".to_string(),
            },
            JmapCall::RemoveKeyword {
                email_id: "email-1".to_string(),
                keyword: "$hail_feed".to_string(),
            },
            JmapCall::RemoveKeyword {
                email_id: "email-1".to_string(),
                keyword: "$hail_papertrail".to_string(),
            },
        ]
    );
    assert_eq!(
        importer.imports()[0].keywords,
        Vec::<String>::new(),
        "provider imports must not pre-classify unknown senders into Imbox before screener routing"
    );
    let route = fixture.intended_route.expect("fixture has route");
    let rule: (String, Option<String>) = sqlx::query_as(
        "SELECT decision, classify_as FROM screener_rules WHERE user_id = ? AND sender_address = ?",
    )
    .bind(user_id)
    .bind(route.sender)
    .fetch_one(&pool)
    .await
    .expect("pending rule");
    assert_eq!(rule, ("pending".to_string(), None));
    let event_type: String = sqlx::query_scalar("SELECT event_type FROM app_events")
        .fetch_one(&pool)
        .await
        .expect("app event");
    assert_eq!(event_type, "screener.pending");
}

#[tokio::test]
async fn routed_import_moves_denied_sender_to_trash() {
    let (pool, _guard, user_id, provider_account_id) = setup().await;
    sqlx::query(
        "INSERT INTO screener_rules \
         (user_id, sender_address, decision, classify_as, decided_at, first_seen_at) \
         VALUES (?, 'denied@example.com', 'deny', NULL, ?, ?)",
    )
    .bind(user_id)
    .bind("2026-01-01T00:00:00Z")
    .bind("2026-01-01T00:00:00Z")
    .execute(&pool)
    .await
    .expect("deny rule");
    let gmail = FakeGmail::new(
        vec![ListMessagesResponse {
            messages: vec![ListMessage {
                id: "gmail-denied".to_owned(),
                thread_id: Some("thread-denied".to_owned()),
            }],
            next_page_token: None,
            result_size_estimate: Some(1),
        }],
        vec![raw_from_message(
            "gmail-denied",
            "thread-denied",
            "history-denied",
            "denied-msg@example.com",
            "Denied <denied@example.com>",
        )],
    );
    let importer = FakeRfc822Importer::default();
    let jmap = FakeJmapOps::default();
    let router = ScreenerRfc822ImportRouter::new(&jmap);
    let routed_importer = RoutingRfc822Importer::new(&importer, &router);

    let summary = import_gmail_history(
        &pool,
        GmailHistoricalImportAccount {
            provider_account_id,
            user_id,
        },
        &gmail,
        &routed_importer,
        GmailHistoricalImportOptions::into_mailboxes(["inbox"]),
        &CancellationToken::new(),
    )
    .await
    .expect("routed import");

    assert_eq!(summary.imported, 1);
    assert_eq!(
        jmap.calls(),
        vec![
            JmapCall::GetMailboxByRole("trash".to_string()),
            JmapCall::MoveToMailbox {
                email_id: "email-1".to_string(),
                mailbox_id: "trash-id".to_string(),
            },
        ]
    );
    let event_type: String = sqlx::query_scalar("SELECT event_type FROM app_events")
        .fetch_one(&pool)
        .await
        .expect("app event");
    assert_eq!(event_type, "thread.updated");
}

fn raw_from_message(
    id: &str,
    thread_id: &str,
    history_id: &str,
    message_id: &str,
    from: &str,
) -> RawGmailMessage {
    RawGmailMessage {
        id: id.to_owned(),
        thread_id: Some(thread_id.to_owned()),
        history_id: Some(history_id.to_owned()),
        label_ids: Vec::new(),
        rfc822: format!(
            "From: {from}\r\nTo: user@example.com\r\nMessage-ID: <{message_id}>\r\nSubject: hi\r\n\r\nBody"
        )
        .into_bytes(),
    }
}
