use hail_db::provider_sync_audit::{
    NewProviderSyncAuditLog, ProviderSyncEventType, ProviderSyncOperationKind,
    ProviderSyncResultStatus, insert_provider_sync_audit_log, list_provider_sync_audit_logs,
};

fn fresh_db_url() -> (String, TempDb) {
    let mut dir = std::env::temp_dir();
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time")
        .as_nanos();

    for attempt in 0..100_u8 {
        dir.push(format!(
            "hail-db-provider-sync-audit-test-{pid}-{nanos}-{attempt}"
        ));
        match std::fs::create_dir(&dir) {
            Ok(()) => {
                let path = dir.join("hail.db");
                let url = format!("sqlite://{}", path.display());
                return (url, TempDb { dir, path });
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                dir.pop();
            }
            Err(err) => panic!("create temp db dir: {err}"),
        }
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

async fn setup() -> (sqlx::SqlitePool, TempDb) {
    let (url, guard) = fresh_db_url();
    let pool = hail_db::connect(&url).await.expect("connect");
    hail_db::migrate(&pool).await.expect("migrate");
    (pool, guard)
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

#[tokio::test]
async fn insert_and_list_provider_sync_audit_logs_for_account() {
    let (pool, _guard) = setup().await;
    let user_id = insert_user(&pool, "audit-user@example.com", "acct-audit-user").await;
    let provider_account_id = insert_provider_account(&pool, user_id, "acct-audit-user").await;

    let first_id = insert_provider_sync_audit_log(
        &pool,
        NewProviderSyncAuditLog {
            user_id,
            provider_account_id,
            operation_kind: ProviderSyncOperationKind::Sync,
            event_type: ProviderSyncEventType::SyncStarted,
            provider_message_id: None,
            result_status: ProviderSyncResultStatus::Started,
            safe_error_code: None,
            safe_error_class: None,
            safe_error_message: None,
            metadata_json: Some(r#"{"page":"initial"}"#),
        },
    )
    .await
    .expect("sync started audit insert");

    let second_id = insert_provider_sync_audit_log(
        &pool,
        NewProviderSyncAuditLog {
            user_id,
            provider_account_id,
            operation_kind: ProviderSyncOperationKind::MessageImport,
            event_type: ProviderSyncEventType::MessageImported,
            provider_message_id: Some("gmail-msg-1"),
            result_status: ProviderSyncResultStatus::Succeeded,
            safe_error_code: None,
            safe_error_class: None,
            safe_error_message: None,
            metadata_json: Some(r#"{"jmapEmailId":"jmap-email-1"}"#),
        },
    )
    .await
    .expect("message imported audit insert");

    assert!(second_id > first_id, "audit ids should increase");

    let logs = list_provider_sync_audit_logs(&pool, user_id, provider_account_id, 10)
        .await
        .expect("list audit logs");

    assert_eq!(logs.len(), 2);
    assert_eq!(logs[0].id, second_id, "newest log should be first");
    assert_eq!(logs[0].user_id, user_id);
    assert_eq!(logs[0].provider_account_id, provider_account_id);
    assert_eq!(logs[0].operation_kind, "message_import");
    assert_eq!(logs[0].event_type, "message_imported");
    assert_eq!(logs[0].provider_message_id.as_deref(), Some("gmail-msg-1"));
    assert_eq!(logs[0].result_status, "succeeded");
    assert_eq!(
        logs[0].metadata_json.as_deref(),
        Some(r#"{"jmapEmailId":"jmap-email-1"}"#)
    );
    assert_eq!(logs[1].id, first_id);
}

#[tokio::test]
async fn audit_logs_support_skips_retries_and_failures() {
    let (pool, _guard) = setup().await;
    let user_id = insert_user(&pool, "states-user@example.com", "acct-states-user").await;
    let provider_account_id = insert_provider_account(&pool, user_id, "acct-states-user").await;

    for (operation_kind, event_type, result_status, message_id, code, class, message) in [
        (
            ProviderSyncOperationKind::MessageSkip,
            ProviderSyncEventType::MessageSkipped,
            ProviderSyncResultStatus::Skipped,
            Some("gmail-msg-skip"),
            None,
            None,
            None,
        ),
        (
            ProviderSyncOperationKind::Retry,
            ProviderSyncEventType::MessageRetryScheduled,
            ProviderSyncResultStatus::Retrying,
            Some("gmail-msg-retry"),
            Some("rate_limited"),
            Some("quota"),
            Some("provider asked us to retry after a redacted delay"),
        ),
        (
            ProviderSyncOperationKind::Failure,
            ProviderSyncEventType::MessageFailed,
            ProviderSyncResultStatus::Failed,
            Some("gmail-msg-fail"),
            Some("malformed_rfc822"),
            Some("permanent"),
            Some("import failed; raw provider response redacted"),
        ),
    ] {
        insert_provider_sync_audit_log(
            &pool,
            NewProviderSyncAuditLog {
                user_id,
                provider_account_id,
                operation_kind,
                event_type,
                provider_message_id: message_id,
                result_status,
                safe_error_code: code,
                safe_error_class: class,
                safe_error_message: message,
                metadata_json: None,
            },
        )
        .await
        .expect("audit insert");
    }

    let logs = list_provider_sync_audit_logs(&pool, user_id, provider_account_id, 10)
        .await
        .expect("list audit logs");

    assert_eq!(logs.len(), 3);
    assert!(logs.iter().any(|log| log.operation_kind == "message_skip"
        && log.event_type == "message_skipped"
        && log.result_status == "skipped"));
    assert!(logs.iter().any(|log| log.operation_kind == "retry"
        && log.event_type == "message_retry_scheduled"
        && log.result_status == "retrying"
        && log.safe_error_code.as_deref() == Some("rate_limited")));
    assert!(logs.iter().any(|log| log.operation_kind == "failure"
        && log.event_type == "message_failed"
        && log.result_status == "failed"
        && log.safe_error_message.as_deref()
            == Some("import failed; raw provider response redacted")));
}

#[tokio::test]
async fn audit_logs_are_scoped_to_provider_account_owner() {
    let (pool, _guard) = setup().await;
    let user_id = insert_user(&pool, "owner@example.com", "acct-owner").await;
    let other_user_id = insert_user(&pool, "other@example.com", "acct-other").await;
    let provider_account_id = insert_provider_account(&pool, user_id, "acct-owner").await;

    let wrong_user = insert_provider_sync_audit_log(
        &pool,
        NewProviderSyncAuditLog {
            user_id: other_user_id,
            provider_account_id,
            operation_kind: ProviderSyncOperationKind::Sync,
            event_type: ProviderSyncEventType::SyncFailed,
            provider_message_id: None,
            result_status: ProviderSyncResultStatus::Failed,
            safe_error_code: Some("auth_revoked"),
            safe_error_class: Some("auth"),
            safe_error_message: Some("provider token revoked"),
            metadata_json: None,
        },
    )
    .await;

    assert!(
        wrong_user.is_err(),
        "audit row user_id must match provider account owner"
    );

    insert_provider_sync_audit_log(
        &pool,
        NewProviderSyncAuditLog {
            user_id,
            provider_account_id,
            operation_kind: ProviderSyncOperationKind::Sync,
            event_type: ProviderSyncEventType::SyncCompleted,
            provider_message_id: None,
            result_status: ProviderSyncResultStatus::Succeeded,
            safe_error_code: None,
            safe_error_class: None,
            safe_error_message: None,
            metadata_json: None,
        },
    )
    .await
    .expect("owner audit insert");

    let visible_to_owner = list_provider_sync_audit_logs(&pool, user_id, provider_account_id, 10)
        .await
        .expect("owner list");
    let visible_to_other =
        list_provider_sync_audit_logs(&pool, other_user_id, provider_account_id, 10)
            .await
            .expect("other list");

    assert_eq!(visible_to_owner.len(), 1);
    assert!(visible_to_other.is_empty());
}
