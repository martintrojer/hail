use sqlx::Row;

use hail_db::provider_message_mappings::{
    DedupedProviderSentCopyMapping, DuplicateProviderMessageMapping, FailedProviderMessageMapping,
    ImportedProviderMessageMapping, LocalSentMessageRef, ProviderImportStatus, ProviderMessageSeen,
    ProviderSentCopyImportDecision, ProviderSentCopyImportInput,
    SENT_COPY_REASON_EXISTING_LOCAL_MESSAGE_ID_MATCH, SENT_COPY_REASON_LOCAL_SENT_MESSAGE_ID_MATCH,
    SENT_COPY_REASON_NO_LOCAL_SENT_MATCH, SENT_COPY_REASON_PROVIDER_MESSAGE_ALREADY_MAPPED,
    SkippedProviderMessageMapping, decide_provider_sent_copy_import,
    find_local_mapping_by_content_sha256, find_local_mapping_by_rfc822_message_id,
    get_provider_message_mapping, list_provider_thread_mappings, mark_provider_message_duplicate,
    mark_provider_message_failed, mark_provider_message_imported,
    mark_provider_message_route_failed, mark_provider_message_skipped,
    mark_provider_sent_copy_deduped, record_provider_message_seen,
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
            "hail-db-provider-message-mappings-test-{pid}-{nanos}-{attempt}"
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
    provider_account_id: &str,
) -> i64 {
    sqlx::query(
        "INSERT INTO mail_accounts \
         (user_id, jmap_account_id, backend_kind, provider_kind, provider_account_id, provider_email, \
          refresh_token_enc, sync_status, created_at, updated_at) \
         VALUES (?, ?, 'gmail', 'gmail', ?, ?, ?, 'active', ?, ?)",
    )
    .bind(user_id)
    .bind(jmap_account_id)
    .bind(provider_account_id)
    .bind(format!("{provider_account_id}@gmail.example"))
    .bind(vec![1_u8; 29])
    .bind("2026-01-01T00:00:00Z")
    .bind("2026-01-01T00:00:00Z")
    .execute(pool)
    .await
    .expect("mail account insert");
    sqlx::query_scalar(
        "SELECT id FROM mail_accounts WHERE user_id = ? AND provider_account_id = ?",
    )
    .bind(user_id)
    .bind(provider_account_id)
    .fetch_one(pool)
    .await
    .expect("mail account id")
}

fn hostile_leak_error() -> &'static str {
    "failed with Authorization: Bearer ya29.mapping-secret access_token=ya29.mapping-secret refresh_token=1//mapping-refresh\r\n\r\nSubject: Mapping Private\r\n\r\nMapping body must not persist"
}

fn assert_no_hostile_leak(surface: &str) {
    for forbidden in [
        "Bearer",
        "ya29.mapping-secret",
        "1//mapping-refresh",
        "Subject: Mapping Private",
        "Mapping body must not persist",
    ] {
        assert!(
            !surface.contains(forbidden),
            "surface leaked {forbidden:?}: {surface}"
        );
    }
}

#[tokio::test]
async fn seen_mapping_is_idempotent_by_provider_message_id() {
    let (pool, _guard) = setup().await;
    let user_id = insert_user(&pool, "mapping-user@example.com", "acct-mapping-user").await;
    let provider_account_id =
        insert_provider_account(&pool, user_id, "acct-mapping-user", "gmail-provider-1").await;

    let first = record_provider_message_seen(
        &pool,
        ProviderMessageSeen {
            provider_account_id,
            provider_message_id: "gmail-msg-1",
            provider_thread_id: Some("gmail-thread-1"),
            provider_history_id: Some("history-1"),
            rfc822_message_id: Some("<msg-1@example.com>"),
            content_sha256: Some(&[1_u8; 32]),
        },
    )
    .await
    .expect("record seen");
    let second = record_provider_message_seen(
        &pool,
        ProviderMessageSeen {
            provider_account_id,
            provider_message_id: "gmail-msg-1",
            provider_thread_id: None,
            provider_history_id: Some("history-2"),
            rfc822_message_id: None,
            content_sha256: None,
        },
    )
    .await
    .expect("record seen retry");

    assert_eq!(second.id, first.id);
    assert_eq!(second.import_status, ProviderImportStatus::Pending);
    assert_eq!(second.provider_thread_id.as_deref(), Some("gmail-thread-1"));
    assert_eq!(second.provider_history_id.as_deref(), Some("history-2"));
    assert_eq!(
        second.rfc822_message_id.as_deref(),
        Some("<msg-1@example.com>")
    );
    assert_eq!(second.content_sha256.as_deref(), Some(&[1_u8; 32][..]));
    let stored = get_provider_message_mapping(&pool, provider_account_id, "gmail-msg-1")
        .await
        .expect("get mapping")
        .expect("mapping exists");
    assert_eq!(stored.id, first.id);
}

#[tokio::test]
async fn failed_mapping_redacts_tokens_and_raw_body_in_error_message() {
    let (pool, _guard) = setup().await;
    let user_id = insert_user(&pool, "failed-redact@example.com", "acct-failed-redact").await;
    let provider_account_id = insert_provider_account(
        &pool,
        user_id,
        "acct-failed-redact",
        "gmail-provider-redact",
    )
    .await;

    let failed = mark_provider_message_failed(
        &pool,
        FailedProviderMessageMapping {
            provider_account_id,
            provider_message_id: "gmail-failed-redact",
            provider_thread_id: Some("thread-redact"),
            provider_history_id: Some("history-redact"),
            rfc822_message_id: Some("<redact@example.com>"),
            content_sha256: Some(&[7_u8; 32]),
            error_class: "hostile_error",
            error_message: Some(hostile_leak_error()),
        },
    )
    .await
    .expect("mark failed");

    assert_eq!(failed.import_status, ProviderImportStatus::Failed);
    let error = failed.error_message.expect("stored error message");
    assert_no_hostile_leak(&error);
    assert!(error.contains("[redacted]"));

    let raw_db_error: String = sqlx::query_scalar(
        "SELECT error_message FROM provider_message_mappings WHERE provider_account_id = ?1 AND provider_message_id = ?2",
    )
    .bind(provider_account_id)
    .bind("gmail-failed-redact")
    .fetch_one(&pool)
    .await
    .expect("raw db error");
    assert_no_hostile_leak(&raw_db_error);
}

#[tokio::test]
async fn imported_mapping_stores_local_jmap_ids_and_is_upsertable() {
    let (pool, _guard) = setup().await;
    let user_id = insert_user(&pool, "import-user@example.com", "acct-import-user").await;
    let provider_account_id =
        insert_provider_account(&pool, user_id, "acct-import-user", "gmail-provider-2").await;

    let pending = record_provider_message_seen(
        &pool,
        ProviderMessageSeen {
            provider_account_id,
            provider_message_id: "gmail-msg-import",
            provider_thread_id: Some("gmail-thread-import"),
            provider_history_id: Some("history-pending"),
            rfc822_message_id: Some("<import@example.com>"),
            content_sha256: None,
        },
    )
    .await
    .expect("pending insert");
    let imported = mark_provider_message_imported(
        &pool,
        ImportedProviderMessageMapping {
            provider_account_id,
            provider_message_id: "gmail-msg-import",
            provider_thread_id: Some("gmail-thread-import"),
            provider_history_id: Some("history-imported"),
            rfc822_message_id: Some("<import@example.com>"),
            content_sha256: Some(&[2_u8; 32]),
            jmap_email_id: "jmap-email-1",
            jmap_thread_id: Some("jmap-thread-1"),
            jmap_mailbox_ids_json: Some(r#"["mailbox-inbox"]"#),
        },
    )
    .await
    .expect("mark imported");

    assert_eq!(imported.id, pending.id);
    assert_eq!(imported.import_status, ProviderImportStatus::Imported);
    assert_eq!(
        imported.provider_history_id.as_deref(),
        Some("history-imported")
    );
    assert_eq!(imported.jmap_email_id.as_deref(), Some("jmap-email-1"));
    assert_eq!(imported.jmap_thread_id.as_deref(), Some("jmap-thread-1"));
    assert_eq!(
        imported.jmap_mailbox_ids_json.as_deref(),
        Some(r#"["mailbox-inbox"]"#)
    );
    assert!(imported.imported_at.is_some());

    let updated = mark_provider_message_imported(
        &pool,
        ImportedProviderMessageMapping {
            provider_account_id,
            provider_message_id: "gmail-msg-import",
            provider_thread_id: None,
            provider_history_id: Some("history-imported-2"),
            rfc822_message_id: None,
            content_sha256: None,
            jmap_email_id: "jmap-email-1b",
            jmap_thread_id: Some("jmap-thread-1b"),
            jmap_mailbox_ids_json: Some(r#"["mailbox-archive"]"#),
        },
    )
    .await
    .expect("mark imported retry");
    assert_eq!(updated.id, pending.id);
    assert_eq!(
        updated.rfc822_message_id.as_deref(),
        Some("<import@example.com>")
    );
    assert_eq!(updated.content_sha256.as_deref(), Some(&[2_u8; 32][..]));
    assert_eq!(updated.jmap_email_id.as_deref(), Some("jmap-email-1b"));
}

#[tokio::test]
async fn route_failed_mapping_keeps_local_ids_and_terminal_import_status() {
    let (pool, _guard) = setup().await;
    let user_id = insert_user(&pool, "route-user@example.com", "acct-route-user").await;
    let provider_account_id =
        insert_provider_account(&pool, user_id, "acct-route-user", "gmail-provider-route").await;

    mark_provider_message_imported(
        &pool,
        ImportedProviderMessageMapping {
            provider_account_id,
            provider_message_id: "gmail-route-failed",
            provider_thread_id: Some("gmail-thread-route"),
            provider_history_id: Some("history-route"),
            rfc822_message_id: Some("route@example.com"),
            content_sha256: Some(&[3_u8; 32]),
            jmap_email_id: "jmap-route-email",
            jmap_thread_id: Some("jmap-route-thread"),
            jmap_mailbox_ids_json: Some(r#"["mailbox-inbox"]"#),
        },
    )
    .await
    .expect("mark imported");

    let route_failed = mark_provider_message_route_failed(
        &pool,
        provider_account_id,
        "gmail-route-failed",
        hostile_leak_error(),
    )
    .await
    .expect("mark route failed");

    assert_eq!(route_failed.import_status, ProviderImportStatus::Imported);
    assert_eq!(
        route_failed.jmap_email_id.as_deref(),
        Some("jmap-route-email")
    );
    assert_eq!(
        route_failed.jmap_thread_id.as_deref(),
        Some("jmap-route-thread")
    );
    assert_eq!(
        route_failed.rfc822_message_id.as_deref(),
        Some("route@example.com")
    );
    assert_eq!(route_failed.error_class.as_deref(), Some("route_import"));
    let error = route_failed.error_message.expect("route error message");
    assert_no_hostile_leak(&error);
    assert!(error.contains("[redacted]"));
}

#[tokio::test]
async fn failed_post_import_mapping_can_converge_without_changing_local_ids() {
    let (pool, _guard) = setup().await;
    let user_id = insert_user(&pool, "crash-window@example.com", "acct-crash-window").await;
    let provider_account_id = insert_provider_account(
        &pool,
        user_id,
        "acct-crash-window",
        "gmail-provider-crash-window",
    )
    .await;

    let failed = mark_provider_message_failed(
        &pool,
        FailedProviderMessageMapping {
            provider_account_id,
            provider_message_id: "gmail-crash-window",
            provider_thread_id: Some("thread-crash-window"),
            provider_history_id: Some("history-before-crash"),
            rfc822_message_id: Some("crash-window@example.com"),
            content_sha256: Some(&[42_u8; 32]),
            error_class: "stalwart_import",
            error_message: Some("simulated crash after local Stalwart import"),
        },
    )
    .await
    .expect("record failed crash-window mapping");
    assert_eq!(failed.import_status, ProviderImportStatus::Failed);
    assert!(failed.jmap_email_id.is_none());

    let imported = mark_provider_message_imported(
        &pool,
        ImportedProviderMessageMapping {
            provider_account_id,
            provider_message_id: "gmail-crash-window",
            provider_thread_id: Some("thread-crash-window"),
            provider_history_id: Some("history-after-retry"),
            rfc822_message_id: Some("crash-window@example.com"),
            content_sha256: Some(&[42_u8; 32]),
            jmap_email_id: "jmap-email-crash-window",
            jmap_thread_id: Some("jmap-thread-crash-window"),
            jmap_mailbox_ids_json: Some(r#"["mailbox-inbox"]"#),
        },
    )
    .await
    .expect("retry converges mapping");

    assert_eq!(imported.id, failed.id);
    assert_eq!(imported.import_status, ProviderImportStatus::Imported);
    assert_eq!(
        imported.provider_history_id.as_deref(),
        Some("history-after-retry")
    );
    assert_eq!(
        imported.rfc822_message_id.as_deref(),
        Some("crash-window@example.com")
    );
    assert_eq!(imported.content_sha256.as_deref(), Some(&[42_u8; 32][..]));
    assert_eq!(
        imported.jmap_email_id.as_deref(),
        Some("jmap-email-crash-window")
    );
    assert!(imported.error_class.is_none());
    assert!(imported.error_message.is_none());
}

#[tokio::test]
async fn content_sha256_must_be_32_bytes() {
    let (pool, _guard) = setup().await;
    let user_id = insert_user(&pool, "hash-user@example.com", "acct-hash-user").await;
    let provider_account_id =
        insert_provider_account(&pool, user_id, "acct-hash-user", "gmail-provider-hash").await;

    let bad = record_provider_message_seen(
        &pool,
        ProviderMessageSeen {
            provider_account_id,
            provider_message_id: "gmail-msg-short-hash",
            provider_thread_id: None,
            provider_history_id: None,
            rfc822_message_id: None,
            content_sha256: Some(&[9_u8; 31]),
        },
    )
    .await;

    assert!(bad.is_err(), "short content_sha256 must fail CHECK");

    let text_hash = sqlx::query(
        "INSERT INTO provider_message_mappings \
         (provider_account_id, provider_message_id, content_sha256, import_status, created_at, updated_at) \
         VALUES (?, 'gmail-msg-text-hash', ?, 'pending', ?, ?)",
    )
    .bind(provider_account_id)
    .bind("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    .bind("2026-01-01T00:00:00Z")
    .bind("2026-01-01T00:00:00Z")
    .execute(&pool)
    .await;

    assert!(text_hash.is_err(), "text content_sha256 must fail CHECK");

    let ok = record_provider_message_seen(
        &pool,
        ProviderMessageSeen {
            provider_account_id,
            provider_message_id: "gmail-msg-valid-hash",
            provider_thread_id: None,
            provider_history_id: None,
            rfc822_message_id: None,
            content_sha256: Some(&[9_u8; 32]),
        },
    )
    .await
    .expect("valid SHA-256 digest accepted");

    assert_eq!(ok.content_sha256.as_deref(), Some(&[9_u8; 32][..]));
}

#[tokio::test]
async fn content_sha256_lookup_uses_account_scoped_partial_index() {
    let (pool, _guard) = setup().await;
    let user_id = insert_user(&pool, "hash-index-user@example.com", "acct-hash-index").await;
    let provider_account_id = insert_provider_account(
        &pool,
        user_id,
        "acct-hash-index",
        "gmail-provider-hash-index",
    )
    .await;

    mark_provider_message_imported(
        &pool,
        ImportedProviderMessageMapping {
            provider_account_id,
            provider_message_id: "gmail-hash-indexed",
            provider_thread_id: None,
            provider_history_id: None,
            rfc822_message_id: None,
            content_sha256: Some(&[10_u8; 32]),
            jmap_email_id: "jmap-hash-indexed",
            jmap_thread_id: None,
            jmap_mailbox_ids_json: None,
        },
    )
    .await
    .expect("seed indexed hash mapping");

    let plan_rows = sqlx::query(
        "EXPLAIN QUERY PLAN \
         SELECT id FROM provider_message_mappings \
         WHERE provider_account_id = ?1 AND content_sha256 = ?2 \
           AND jmap_email_id IS NOT NULL AND import_status IN ('imported', 'duplicate') \
         ORDER BY CASE import_status WHEN 'imported' THEN 0 ELSE 1 END, id ASC \
         LIMIT 1",
    )
    .bind(provider_account_id)
    .bind(vec![10_u8; 32])
    .fetch_all(&pool)
    .await
    .expect("query plan");
    let plan = plan_rows
        .iter()
        .map(|row| row.get::<String, _>("detail"))
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        plan.contains("idx_provider_message_mappings_content_sha256"),
        "content digest dedupe lookup must use content_sha256 index; plan was {plan}"
    );
}

#[tokio::test]
async fn rfc822_message_id_finds_existing_local_mapping_within_account_only() {
    let (pool, _guard) = setup().await;
    let user_id = insert_user(&pool, "rfc822-user@example.com", "acct-rfc822-user").await;
    let account_a = insert_provider_account(&pool, user_id, "acct-rfc822-user", "gmail-a").await;
    let account_b = insert_provider_account(&pool, user_id, "acct-rfc822-user", "gmail-b").await;

    record_provider_message_seen(
        &pool,
        ProviderMessageSeen {
            provider_account_id: account_a,
            provider_message_id: "pending-same-rfc822",
            provider_thread_id: Some("thread-pending"),
            provider_history_id: None,
            rfc822_message_id: Some("<same@example.com>"),
            content_sha256: None,
        },
    )
    .await
    .expect("pending insert");
    assert!(
        find_local_mapping_by_rfc822_message_id(&pool, account_a, "<same@example.com>")
            .await
            .expect("lookup pending")
            .is_none()
    );

    let imported = mark_provider_message_imported(
        &pool,
        ImportedProviderMessageMapping {
            provider_account_id: account_a,
            provider_message_id: "gmail-imported-same-rfc822",
            provider_thread_id: Some("thread-imported"),
            provider_history_id: Some("history-imported"),
            rfc822_message_id: Some("<same@example.com>"),
            content_sha256: Some(&[3_u8; 32]),
            jmap_email_id: "jmap-existing",
            jmap_thread_id: Some("jmap-thread-existing"),
            jmap_mailbox_ids_json: Some(r#"["mailbox-inbox"]"#),
        },
    )
    .await
    .expect("imported insert");
    let found = find_local_mapping_by_rfc822_message_id(&pool, account_a, "<same@example.com>")
        .await
        .expect("lookup imported")
        .expect("imported hit");
    assert_eq!(found.id, imported.id);
    assert_eq!(found.jmap_email_id.as_deref(), Some("jmap-existing"));
    let found_by_content = find_local_mapping_by_content_sha256(&pool, account_a, &[3_u8; 32])
        .await
        .expect("content lookup imported")
        .expect("content hit");
    assert_eq!(found_by_content.id, imported.id);
    assert_eq!(
        found_by_content.jmap_email_id.as_deref(),
        Some("jmap-existing")
    );
    assert!(
        find_local_mapping_by_rfc822_message_id(&pool, account_b, "<same@example.com>")
            .await
            .expect("lookup other account")
            .is_none()
    );
    assert!(
        find_local_mapping_by_content_sha256(&pool, account_b, &[3_u8; 32])
            .await
            .expect("content lookup other account")
            .is_none()
    );
}

#[tokio::test]
async fn duplicate_mapping_records_provider_copy_without_new_import() {
    let (pool, _guard) = setup().await;
    let user_id = insert_user(&pool, "dupe-user@example.com", "acct-dupe-user").await;
    let provider_account_id =
        insert_provider_account(&pool, user_id, "acct-dupe-user", "gmail-provider-dupe").await;

    mark_provider_message_imported(
        &pool,
        ImportedProviderMessageMapping {
            provider_account_id,
            provider_message_id: "gmail-original",
            provider_thread_id: Some("gmail-thread-dupe"),
            provider_history_id: Some("history-original"),
            rfc822_message_id: Some("<dupe@example.com>"),
            content_sha256: Some(&[4_u8; 32]),
            jmap_email_id: "jmap-original",
            jmap_thread_id: Some("jmap-thread-original"),
            jmap_mailbox_ids_json: Some(r#"["mailbox-sent"]"#),
        },
    )
    .await
    .expect("original import");
    let existing =
        find_local_mapping_by_rfc822_message_id(&pool, provider_account_id, "<dupe@example.com>")
            .await
            .expect("rfc822 lookup")
            .expect("existing local mapping");
    let duplicate = mark_provider_message_duplicate(
        &pool,
        DuplicateProviderMessageMapping {
            provider_account_id,
            provider_message_id: "gmail-provider-sent-copy",
            provider_thread_id: Some("gmail-thread-dupe"),
            provider_history_id: Some("history-copy"),
            rfc822_message_id: Some("<dupe@example.com>"),
            content_sha256: Some(&[4_u8; 32]),
            duplicate_jmap_email_id: existing.jmap_email_id.as_deref(),
            duplicate_jmap_thread_id: existing.jmap_thread_id.as_deref(),
            duplicate_jmap_mailbox_ids_json: existing.jmap_mailbox_ids_json.as_deref(),
        },
    )
    .await
    .expect("mark duplicate");

    assert_eq!(duplicate.import_status, ProviderImportStatus::Duplicate);
    assert_eq!(duplicate.jmap_email_id.as_deref(), Some("jmap-original"));
    assert!(duplicate.imported_at.is_none());
    let thread_mappings =
        list_provider_thread_mappings(&pool, provider_account_id, "gmail-thread-dupe")
            .await
            .expect("list thread mappings");
    assert_eq!(thread_mappings.len(), 2);
    assert_eq!(thread_mappings[0].provider_message_id, "gmail-original");
    assert_eq!(
        thread_mappings[1].provider_message_id,
        "gmail-provider-sent-copy"
    );
}

#[tokio::test]
async fn provider_sent_copy_policy_dedupes_against_local_sent_message_id() {
    let (pool, _guard) = setup().await;
    let user_id = insert_user(&pool, "sent-copy-user@example.com", "acct-sent-copy-user").await;
    let provider_account_id =
        insert_provider_account(&pool, user_id, "acct-sent-copy-user", "gmail-provider-sent").await;

    let decision = decide_provider_sent_copy_import(
        &pool,
        ProviderSentCopyImportInput {
            provider_account_id,
            provider_message_id: "gmail-sent-copy-1",
            rfc822_message_id: Some("<local-sent@example.com>"),
            local_sent: Some(LocalSentMessageRef {
                rfc822_message_id: "<local-sent@example.com>",
                jmap_email_id: "jmap-local-sent",
                jmap_thread_id: Some("jmap-thread-sent"),
                jmap_mailbox_ids_json: Some(r#"["mailbox-sent"]"#),
            }),
        },
    )
    .await
    .expect("sent copy decision");

    let ProviderSentCopyImportDecision::DeduplicateToLocal {
        jmap_email_id,
        jmap_thread_id,
        jmap_mailbox_ids_json,
        reason_class,
    } = decision
    else {
        panic!("expected local sent dedupe decision");
    };
    assert_eq!(reason_class, SENT_COPY_REASON_LOCAL_SENT_MESSAGE_ID_MATCH);
    assert_eq!(jmap_email_id, "jmap-local-sent");
    assert_eq!(jmap_thread_id.as_deref(), Some("jmap-thread-sent"));

    let mapping = mark_provider_sent_copy_deduped(
        &pool,
        DedupedProviderSentCopyMapping {
            provider_account_id,
            provider_message_id: "gmail-sent-copy-1",
            provider_thread_id: Some("gmail-thread-sent"),
            provider_history_id: Some("history-sent"),
            rfc822_message_id: Some("<local-sent@example.com>"),
            content_sha256: Some(&[7_u8; 32]),
            duplicate_jmap_email_id: &jmap_email_id,
            duplicate_jmap_thread_id: jmap_thread_id.as_deref(),
            duplicate_jmap_mailbox_ids_json: jmap_mailbox_ids_json.as_deref(),
            reason_class,
            reason_message: Some("provider Sent copy matched local Stalwart Sent Message-ID"),
        },
    )
    .await
    .expect("mark sent copy deduped");

    assert_eq!(mapping.import_status, ProviderImportStatus::Duplicate);
    assert_eq!(mapping.jmap_email_id.as_deref(), Some("jmap-local-sent"));
    assert_eq!(mapping.error_class.as_deref(), Some(reason_class));
    assert_eq!(mapping.imported_at, None);

    let retry_decision = decide_provider_sent_copy_import(
        &pool,
        ProviderSentCopyImportInput {
            provider_account_id,
            provider_message_id: "gmail-sent-copy-1",
            rfc822_message_id: Some("<local-sent@example.com>"),
            local_sent: None,
        },
    )
    .await
    .expect("retry sent copy decision");
    let ProviderSentCopyImportDecision::SkipAlreadyMapped {
        existing,
        reason_class,
    } = retry_decision
    else {
        panic!("expected already mapped skip decision");
    };
    assert_eq!(existing.id, mapping.id);
    assert_eq!(
        reason_class,
        SENT_COPY_REASON_PROVIDER_MESSAGE_ALREADY_MAPPED
    );
}

#[tokio::test]
async fn provider_sent_copy_policy_uses_existing_message_id_mapping_before_importing() {
    let (pool, _guard) = setup().await;
    let user_id = insert_user(
        &pool,
        "sent-copy-existing@example.com",
        "acct-sent-existing",
    )
    .await;
    let provider_account_id = insert_provider_account(
        &pool,
        user_id,
        "acct-sent-existing",
        "gmail-provider-existing",
    )
    .await;

    mark_provider_message_imported(
        &pool,
        ImportedProviderMessageMapping {
            provider_account_id,
            provider_message_id: "gmail-localized-original",
            provider_thread_id: Some("gmail-thread-existing"),
            provider_history_id: Some("history-existing"),
            rfc822_message_id: Some("<existing-local@example.com>"),
            content_sha256: Some(&[8_u8; 32]),
            jmap_email_id: "jmap-existing-local",
            jmap_thread_id: Some("jmap-thread-existing"),
            jmap_mailbox_ids_json: Some(r#"["mailbox-sent"]"#),
        },
    )
    .await
    .expect("seed existing mapping");

    let decision = decide_provider_sent_copy_import(
        &pool,
        ProviderSentCopyImportInput {
            provider_account_id,
            provider_message_id: "gmail-sent-copy-existing",
            rfc822_message_id: Some("<existing-local@example.com>"),
            local_sent: None,
        },
    )
    .await
    .expect("sent copy decision");

    let ProviderSentCopyImportDecision::DeduplicateToLocal {
        jmap_email_id,
        reason_class,
        ..
    } = decision
    else {
        panic!("expected existing mapping dedupe decision");
    };
    assert_eq!(
        reason_class,
        SENT_COPY_REASON_EXISTING_LOCAL_MESSAGE_ID_MATCH
    );
    assert_eq!(jmap_email_id, "jmap-existing-local");

    let unmatched = decide_provider_sent_copy_import(
        &pool,
        ProviderSentCopyImportInput {
            provider_account_id,
            provider_message_id: "gmail-unmatched-sent-copy",
            rfc822_message_id: Some("<provider-only@example.com>"),
            local_sent: None,
        },
    )
    .await
    .expect("unmatched sent copy decision");
    assert_eq!(
        unmatched,
        ProviderSentCopyImportDecision::ImportAsProviderMessage {
            reason_class: SENT_COPY_REASON_NO_LOCAL_SENT_MATCH,
        }
    );
}

#[tokio::test]
async fn failed_and_skipped_statuses_store_safe_reason_fields() {
    let (pool, _guard) = setup().await;
    let user_id = insert_user(&pool, "states-user@example.com", "acct-states-user").await;
    let provider_account_id =
        insert_provider_account(&pool, user_id, "acct-states-user", "gmail-provider-states").await;

    let failed = mark_provider_message_failed(
        &pool,
        FailedProviderMessageMapping {
            provider_account_id,
            provider_message_id: "gmail-failed",
            provider_thread_id: Some("gmail-thread-failed"),
            provider_history_id: Some("history-failed"),
            rfc822_message_id: Some("<failed@example.com>"),
            content_sha256: Some(&[5_u8; 32]),
            error_class: "malformed_rfc822",
            error_message: Some("raw body rejected; contents redacted"),
        },
    )
    .await
    .expect("mark failed");
    assert_eq!(failed.import_status, ProviderImportStatus::Failed);
    assert_eq!(failed.error_class.as_deref(), Some("malformed_rfc822"));
    assert!(failed.jmap_email_id.is_none());

    let skipped = mark_provider_message_skipped(
        &pool,
        SkippedProviderMessageMapping {
            provider_account_id,
            provider_message_id: "gmail-skipped",
            provider_thread_id: Some("gmail-thread-skipped"),
            provider_history_id: Some("history-skipped"),
            rfc822_message_id: Some("<skipped@example.com>"),
            content_sha256: Some(&[6_u8; 32]),
            reason_class: "label_excluded",
            reason_message: Some("provider spam/trash import disabled"),
        },
    )
    .await
    .expect("mark skipped");
    assert_eq!(skipped.import_status, ProviderImportStatus::Skipped);
    assert_eq!(skipped.error_class.as_deref(), Some("label_excluded"));
    assert!(skipped.jmap_email_id.is_none());
}
