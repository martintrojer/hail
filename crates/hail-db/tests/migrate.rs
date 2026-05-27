//! Integration tests for the baseline migration (design.md §6.2).

use sqlx::Row;

/// All application-owned tables defined in §6.2 (v1 baseline). The
/// `_sqlx_migrations` table is managed by sqlx itself but is asserted on too
/// because we want to be sure the migration ran.
const EXPECTED_TABLES: &[&str] = &[
    "_sqlx_migrations",
    "users",
    "sessions",
    "screener_rules",
    "contact_notes",
    "stack_positions",
    "bubble_ups",
    "scheduled_sends",
    "user_prefs",
    "jmap_state",
    "audit_log",
    "undo_actions",
    "app_events",
    "thread_notes",
    "thread_seen",
    "workflow_rules",
    "user_invites",
    "provider_accounts",
    "provider_oauth_states",
    "provider_message_mappings",
    "provider_sync_events",
    "labels",
    "thread_labels",
    "speakeasy_passphrases",
];

/// Indices explicitly declared in §6.2. Partial indices count as well.
const EXPECTED_INDICES: &[&str] = &[
    "idx_sessions_user",
    "idx_stack_order",
    "idx_bubble_ups_pending",
    "idx_scheduled_sends_due",
    "idx_undo_actions_user_live",
    "idx_app_events_id",
    "idx_app_events_user_id",
    "idx_thread_notes_thread",
    "idx_thread_seen_user",
    "idx_workflow_rules_user",
    "idx_user_invites_token_hash",
    "idx_user_invites_email",
    "idx_user_invites_pending",
    "idx_provider_accounts_user",
    "idx_provider_accounts_status",
    "idx_provider_accounts_provider_email",
    "idx_provider_oauth_states_user",
    "idx_provider_message_mappings_thread",
    "idx_provider_message_mappings_rfc822",
    "idx_provider_message_mappings_jmap_email",
    "idx_provider_message_mappings_status",
    "idx_provider_message_mappings_content_sha256",
    "idx_provider_sync_events_account_time",
    "idx_provider_sync_events_type",
    "idx_provider_sync_events_user_account_time",
    "idx_provider_sync_events_account_result",
    "idx_labels_user_id",
    "idx_labels_user_normalized_name",
    "idx_labels_provider_identity",
    "idx_labels_user_name",
    "idx_thread_labels_label",
    "idx_speakeasy_passphrases_rotates_at",
];

const EXPECTED_SCHEDULED_SEND_COLUMNS: &[&str] = &[
    "id",
    "user_id",
    "draft_email_id",
    "send_at",
    "status",
    "auth_session_id",
    "auth_session_expires_at",
    "claimed_at",
    "sent_at",
    "error",
    "created_at",
];

/// Build a fresh DB URL backed by a unique temp directory. We deliberately avoid
/// `sqlite::memory:` because a multi-connection pool gives each connection
/// its own private in-memory DB, which makes migrations invisible to later
/// queries. A scratch file is also closer to the production code path
/// (WAL + sync NORMAL + foreign_keys).
fn fresh_db_url() -> (String, TempDb) {
    let mut dir = std::env::temp_dir();
    let pid = std::process::id();
    let thread = format!("{:?}", std::thread::current().id());
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time")
        .as_nanos();

    for attempt in 0..100_u8 {
        dir.push(format!(
            "hail-db-migrate-test-{pid}-{thread}-{nanos}-{attempt}"
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

#[tokio::test]
async fn connect_and_migrate_succeeds_in_memory() {
    // Per task contract: open `sqlite::memory:`, run connect + migrate, expect
    // no error. Note that each pool connection to `:memory:` gets its own
    // private DB, so we only assert the migration drive ran cleanly here; the
    // table/index/constraint asserts below use a file-backed DB so post-
    // migration queries see the schema.
    let pool = hail_db::connect("sqlite::memory:").await.expect("connect");
    hail_db::migrate(&pool).await.expect("migrate");
}

#[tokio::test]
async fn connect_and_migrate_succeeds_on_file() {
    let (_pool, _guard) = setup().await;
}

#[tokio::test]
async fn all_baseline_tables_exist() {
    let (pool, _guard) = setup().await;

    let rows = sqlx::query("SELECT name FROM sqlite_master WHERE type = 'table'")
        .fetch_all(&pool)
        .await
        .expect("query sqlite_master");

    let names: std::collections::HashSet<String> =
        rows.iter().map(|r| r.get::<String, _>("name")).collect();

    for table in EXPECTED_TABLES {
        assert!(
            names.contains(*table),
            "expected table `{table}` missing; got {names:?}"
        );
    }
}

#[tokio::test]
async fn all_baseline_indices_exist() {
    let (pool, _guard) = setup().await;

    let rows = sqlx::query("SELECT name FROM sqlite_master WHERE type = 'index'")
        .fetch_all(&pool)
        .await
        .expect("query sqlite_master");

    let names: std::collections::HashSet<String> =
        rows.iter().map(|r| r.get::<String, _>("name")).collect();

    for idx in EXPECTED_INDICES {
        assert!(
            names.contains(*idx),
            "expected index `{idx}` missing; got {names:?}"
        );
    }
}

#[tokio::test]
async fn v1_1_tables_are_not_present() {
    // Guard: §6.3 tables (email_notes, clips) must NOT be in the v1 baseline.
    let (pool, _guard) = setup().await;
    let rows = sqlx::query("SELECT name FROM sqlite_master WHERE type = 'table'")
        .fetch_all(&pool)
        .await
        .expect("query sqlite_master");
    let names: std::collections::HashSet<String> =
        rows.iter().map(|r| r.get::<String, _>("name")).collect();
    assert!(
        !names.contains("email_notes"),
        "v1.1 table leaked into baseline"
    );
    assert!(!names.contains("clips"), "v1.1 table leaked into baseline");
}

#[tokio::test]
async fn users_email_is_unique() {
    let (pool, _guard) = setup().await;

    sqlx::query("INSERT INTO users (email, jmap_account_id, created_at) VALUES (?, ?, ?)")
        .bind("alice@example.com")
        .bind("acct-1")
        .bind("2026-01-01T00:00:00Z")
        .execute(&pool)
        .await
        .expect("first insert");

    let dup =
        sqlx::query("INSERT INTO users (email, jmap_account_id, created_at) VALUES (?, ?, ?)")
            .bind("alice@example.com")
            .bind("acct-2")
            .bind("2026-01-02T00:00:00Z")
            .execute(&pool)
            .await;

    assert!(
        dup.is_err(),
        "duplicate email must violate UNIQUE constraint"
    );
}

#[tokio::test]
async fn screener_rules_decision_check_enforced() {
    let (pool, _guard) = setup().await;

    sqlx::query("INSERT INTO users (email, jmap_account_id, created_at) VALUES (?, ?, ?)")
        .bind("bob@example.com")
        .bind("acct-bob")
        .bind("2026-01-01T00:00:00Z")
        .execute(&pool)
        .await
        .expect("user insert");

    let user_id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE email = ?")
        .bind("bob@example.com")
        .fetch_one(&pool)
        .await
        .expect("fetch user id");

    let bad = sqlx::query(
        "INSERT INTO screener_rules (user_id, sender_address, decision, first_seen_at) \
         VALUES (?, ?, ?, ?)",
    )
    .bind(user_id)
    .bind("spam@example.com")
    .bind("maybe") // not in ('allow','deny','pending')
    .bind("2026-01-01T00:00:00Z")
    .execute(&pool)
    .await;

    assert!(
        bad.is_err(),
        "CHECK constraint on decision must reject `maybe`"
    );
}

#[tokio::test]
async fn scheduled_sends_has_processing_claim_schema() {
    let (pool, _guard) = setup().await;

    let rows = sqlx::query("PRAGMA table_info(scheduled_sends)")
        .fetch_all(&pool)
        .await
        .expect("scheduled_sends table_info");
    let columns: std::collections::HashSet<String> =
        rows.iter().map(|r| r.get::<String, _>("name")).collect();

    for column in EXPECTED_SCHEDULED_SEND_COLUMNS {
        assert!(
            columns.contains(*column),
            "expected scheduled_sends column `{column}` missing; got {columns:?}"
        );
    }

    sqlx::query("INSERT INTO users (email, jmap_account_id, created_at) VALUES (?, ?, ?)")
        .bind("schedule@example.com")
        .bind("acct-schedule")
        .bind("2026-01-01T00:00:00Z")
        .execute(&pool)
        .await
        .expect("user insert");

    let user_id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE email = ?")
        .bind("schedule@example.com")
        .fetch_one(&pool)
        .await
        .expect("fetch user id");

    sqlx::query(
        "INSERT INTO scheduled_sends \
         (user_id, draft_email_id, send_at, status, claimed_at, created_at) \
         VALUES (?, ?, ?, 'processing', ?, ?)",
    )
    .bind(user_id)
    .bind("draft-processing")
    .bind("2026-01-02T00:00:00Z")
    .bind("2026-01-02T00:00:01Z")
    .bind("2026-01-01T00:00:00Z")
    .execute(&pool)
    .await
    .expect("processing scheduled_send should satisfy status check");

    sqlx::query(
        "INSERT INTO sessions \
         (id, user_id, jmap_token_enc, expires_at, created_at, last_used_at) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    .bind(user_id)
    .bind(vec![0u8; 16])
    .bind("2026-02-01T00:00:00Z")
    .bind("2026-01-01T00:00:00Z")
    .bind("2026-01-01T00:00:00Z")
    .execute(&pool)
    .await
    .expect("session insert");

    sqlx::query(
        "INSERT INTO scheduled_sends \
         (user_id, draft_email_id, send_at, status, auth_session_id, auth_session_expires_at, created_at) \
         VALUES (?, ?, ?, 'auth_required', ?, ?, ?)",
    )
    .bind(user_id)
    .bind("draft-auth-required")
    .bind("2026-01-02T00:00:00Z")
    .bind("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    .bind("2026-02-01T00:00:00Z")
    .bind("2026-01-01T00:00:00Z")
    .execute(&pool)
    .await
    .expect("auth_required scheduled_send should satisfy status check and session reference");
}

#[tokio::test]
async fn thread_notes_cascade_when_user_is_deleted() {
    let (pool, _guard) = setup().await;

    sqlx::query("INSERT INTO users (email, jmap_account_id, created_at) VALUES (?, ?, ?)")
        .bind("notes-cascade@example.com")
        .bind("acct-notes-cascade")
        .bind("2026-01-01T00:00:00Z")
        .execute(&pool)
        .await
        .expect("user insert");

    let user_id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE email = ?")
        .bind("notes-cascade@example.com")
        .fetch_one(&pool)
        .await
        .expect("fetch user id");

    sqlx::query(
        "INSERT INTO thread_notes (user_id, thread_id, email_id, body) VALUES (?, ?, ?, ?)",
    )
    .bind(user_id)
    .bind("thread-1")
    .bind("email-1")
    .bind("remember this")
    .execute(&pool)
    .await
    .expect("thread note insert");

    sqlx::query("DELETE FROM users WHERE id = ?")
        .bind(user_id)
        .execute(&pool)
        .await
        .expect("user delete cascades");

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM thread_notes WHERE user_id = ?")
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .expect("thread note count");
    assert_eq!(
        count, 0,
        "thread_notes must be user-scoped with ON DELETE CASCADE"
    );
}

#[tokio::test]
async fn provider_accounts_capture_oauth_and_sync_state() {
    let (pool, _guard) = setup().await;

    sqlx::query("INSERT INTO users (email, jmap_account_id, created_at) VALUES (?, ?, ?)")
        .bind("gmail-user@example.com")
        .bind("acct-gmail-user")
        .bind("2026-01-01T00:00:00Z")
        .execute(&pool)
        .await
        .expect("user insert");

    let user_id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE email = ?")
        .bind("gmail-user@example.com")
        .fetch_one(&pool)
        .await
        .expect("fetch user id");

    sqlx::query(
        "INSERT INTO provider_accounts \
         (user_id, jmap_account_id, provider_kind, provider_account_id, provider_email, display_email, \
          granted_scopes_json, consented_at, refresh_token_enc, refresh_token_key_id, \
          cached_access_token_expires_at, access_token_refreshed_at, last_profile_history_id, \
          profile_synced_at, initial_sync_completed_at, sync_status, backfill_cursor_json, last_sync_attempted_at, \
          last_sync_succeeded_at, created_at, updated_at) \
         VALUES (?, ?, 'gmail', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'active', ?, ?, ?, ?, ?)",
    )
    .bind(user_id)
    .bind("acct-gmail-user")
    .bind("gmail-provider-id-1")
    .bind("gmail-user@gmail.com")
    .bind("Gmail User <gmail-user@gmail.com>")
    .bind(r#"["https://www.googleapis.com/auth/gmail.readonly"]"#)
    .bind("2026-01-01T00:00:00Z")
    .bind(vec![1_u8; 29])
    .bind("server-key-v1")
    .bind("2026-01-01T01:00:00Z")
    .bind("2026-01-01T00:30:00Z")
    .bind("history-123")
    .bind("2026-01-01T00:30:00Z")
    .bind("2026-01-01T00:45:00Z")
    .bind(r#"{"pageToken":"abc"}"#)
    .bind("2026-01-01T00:40:00Z")
    .bind("2026-01-01T00:45:00Z")
    .bind("2026-01-01T00:00:00Z")
    .bind("2026-01-01T00:45:00Z")
    .execute(&pool)
    .await
    .expect("provider account insert");

    let bad_kind = sqlx::query(
        "INSERT INTO provider_accounts \
         (user_id, jmap_account_id, provider_kind, provider_account_id, provider_email, \
          refresh_token_enc, sync_status, created_at, updated_at) \
         VALUES (?, ?, 'imap', 'imap-1', 'imap@example.com', ?, 'active', ?, ?)",
    )
    .bind(user_id)
    .bind("acct-gmail-user")
    .bind(vec![9_u8; 29])
    .bind("2026-01-01T00:00:00Z")
    .bind("2026-01-01T00:00:00Z")
    .execute(&pool)
    .await;
    assert!(bad_kind.is_err(), "provider kind must be constrained");

    let missing_token = sqlx::query(
        "INSERT INTO provider_accounts \
         (user_id, jmap_account_id, provider_kind, provider_account_id, provider_email, \
          sync_status, created_at, updated_at) \
         VALUES (?, ?, 'gmail', 'gmail-provider-id-2', 'other@gmail.com', 'active', ?, ?)",
    )
    .bind(user_id)
    .bind("acct-gmail-user")
    .bind("2026-01-01T00:00:00Z")
    .bind("2026-01-01T00:00:00Z")
    .execute(&pool)
    .await;
    assert!(
        missing_token.is_err(),
        "active provider account must have encrypted token material"
    );

    let short_token = sqlx::query(
        "INSERT INTO provider_accounts \
         (user_id, jmap_account_id, provider_kind, provider_account_id, provider_email, \
          refresh_token_enc, sync_status, created_at, updated_at) \
         VALUES (?, ?, 'gmail', 'gmail-provider-short-token', 'short-token@gmail.com', ?, 'active', ?, ?)",
    )
    .bind(user_id)
    .bind("acct-gmail-user")
    .bind(vec![7_u8; 28])
    .bind("2026-01-01T00:00:00Z")
    .bind("2026-01-01T00:00:00Z")
    .execute(&pool)
    .await;
    assert!(
        short_token.is_err(),
        "active provider account must reject empty/too-short encrypted token material"
    );

    let token_ref = sqlx::query(
        "INSERT INTO provider_accounts \
         (user_id, jmap_account_id, provider_kind, provider_account_id, provider_email, \
          refresh_token_ref, sync_status, created_at, updated_at) \
         VALUES (?, ?, 'gmail', 'gmail-provider-ref-token', 'ref-token@gmail.com', ?, 'active', ?, ?)",
    )
    .bind(user_id)
    .bind("acct-gmail-user")
    .bind("kms://hail/provider-token/1")
    .bind("2026-01-01T00:00:00Z")
    .bind("2026-01-01T00:00:00Z")
    .execute(&pool)
    .await;
    assert!(
        token_ref.is_err(),
        "active provider account must reject unresolved external token references"
    );

    let bad_granted_scopes = sqlx::query(
        "INSERT INTO provider_accounts \
         (user_id, jmap_account_id, provider_kind, provider_account_id, provider_email, \
          granted_scopes_json, refresh_token_enc, sync_status, created_at, updated_at) \
         VALUES (?, ?, 'gmail', 'gmail-provider-bad-scopes', 'bad-scopes@gmail.com', ?, ?, 'active', ?, ?)",
    )
    .bind(user_id)
    .bind("acct-gmail-user")
    .bind("not-json")
    .bind(vec![5_u8; 29])
    .bind("2026-01-01T00:00:00Z")
    .bind("2026-01-01T00:00:00Z")
    .execute(&pool)
    .await;
    assert!(
        bad_granted_scopes.is_err(),
        "granted_scopes_json must be valid JSON"
    );

    let bad_backfill_cursor = sqlx::query(
        "UPDATE provider_accounts SET backfill_cursor_json = ? WHERE provider_account_id = 'gmail-provider-id-1'",
    )
    .bind("{not-json")
    .execute(&pool)
    .await;
    assert!(
        bad_backfill_cursor.is_err(),
        "backfill_cursor_json must be valid JSON when present"
    );

    let duplicate = sqlx::query(
        "INSERT INTO provider_accounts \
         (user_id, jmap_account_id, provider_kind, provider_account_id, provider_email, \
          refresh_token_enc, sync_status, created_at, updated_at) \
         VALUES (?, ?, 'gmail', 'gmail-provider-id-1', 'dupe@gmail.com', ?, 'active', ?, ?)",
    )
    .bind(user_id)
    .bind("acct-gmail-user")
    .bind(vec![5_u8; 29])
    .bind("2026-01-01T00:00:00Z")
    .bind("2026-01-01T00:00:00Z")
    .execute(&pool)
    .await;
    assert!(
        duplicate.is_err(),
        "provider account identity must be unique per user/provider"
    );
}

#[tokio::test]
async fn provider_message_mappings_are_idempotent_and_audited() {
    let (pool, _guard) = setup().await;

    sqlx::query("INSERT INTO users (email, jmap_account_id, created_at) VALUES (?, ?, ?)")
        .bind("mapping-user@example.com")
        .bind("acct-mapping-user")
        .bind("2026-01-01T00:00:00Z")
        .execute(&pool)
        .await
        .expect("user insert");

    let user_id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE email = ?")
        .bind("mapping-user@example.com")
        .fetch_one(&pool)
        .await
        .expect("fetch user id");

    sqlx::query(
        "INSERT INTO provider_accounts \
         (user_id, jmap_account_id, provider_kind, provider_account_id, provider_email, \
          refresh_token_enc, sync_status, created_at, updated_at) \
         VALUES (?, ?, 'gmail', 'gmail-provider-id-1', 'mapping@gmail.com', ?, 'active', ?, ?)",
    )
    .bind(user_id)
    .bind("acct-mapping-user")
    .bind(vec![8_u8; 29])
    .bind("2026-01-01T00:00:00Z")
    .bind("2026-01-01T00:00:00Z")
    .execute(&pool)
    .await
    .expect("provider account insert");

    let account_id: i64 = sqlx::query_scalar("SELECT id FROM provider_accounts WHERE user_id = ?")
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .expect("provider account id");

    sqlx::query(
        "INSERT INTO provider_message_mappings \
         (provider_account_id, provider_message_id, provider_thread_id, provider_history_id, \
          rfc822_message_id, content_sha256, jmap_email_id, jmap_thread_id, jmap_mailbox_ids_json, \
          import_status, imported_at, last_seen_at, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'imported', ?, ?, ?, ?)",
    )
    .bind(account_id)
    .bind("gmail-msg-1")
    .bind("gmail-thread-1")
    .bind("history-456")
    .bind("<message-1@example.com>")
    .bind(vec![7_u8; 32])
    .bind("jmap-email-1")
    .bind("jmap-thread-1")
    .bind(r#"["mailbox-inbox"]"#)
    .bind("2026-01-01T00:10:00Z")
    .bind("2026-01-01T00:10:00Z")
    .bind("2026-01-01T00:00:00Z")
    .bind("2026-01-01T00:10:00Z")
    .execute(&pool)
    .await
    .expect("mapping insert");

    let duplicate = sqlx::query(
        "INSERT INTO provider_message_mappings \
         (provider_account_id, provider_message_id, import_status, created_at, updated_at) \
         VALUES (?, 'gmail-msg-1', 'pending', ?, ?)",
    )
    .bind(account_id)
    .bind("2026-01-01T00:11:00Z")
    .bind("2026-01-01T00:11:00Z")
    .execute(&pool)
    .await;
    assert!(
        duplicate.is_err(),
        "provider message id must be the primary import idempotency key"
    );

    let bad_status = sqlx::query(
        "INSERT INTO provider_message_mappings \
         (provider_account_id, provider_message_id, import_status, created_at, updated_at) \
         VALUES (?, 'gmail-msg-2', 'maybe', ?, ?)",
    )
    .bind(account_id)
    .bind("2026-01-01T00:11:00Z")
    .bind("2026-01-01T00:11:00Z")
    .execute(&pool)
    .await;
    assert!(bad_status.is_err(), "import status must be constrained");

    let bad_content_hash = sqlx::query(
        "INSERT INTO provider_message_mappings \
         (provider_account_id, provider_message_id, content_sha256, import_status, created_at, updated_at) \
         VALUES (?, 'gmail-msg-bad-hash', ?, 'pending', ?, ?)",
    )
    .bind(account_id)
    .bind(vec![8_u8; 31])
    .bind("2026-01-01T00:11:00Z")
    .bind("2026-01-01T00:11:00Z")
    .execute(&pool)
    .await;
    assert!(
        bad_content_hash.is_err(),
        "content_sha256 must be constrained to 32-byte SHA-256 digests"
    );

    let text_content_hash = sqlx::query(
        "INSERT INTO provider_message_mappings \
         (provider_account_id, provider_message_id, content_sha256, import_status, created_at, updated_at) \
         VALUES (?, 'gmail-msg-text-hash', ?, 'pending', ?, ?)",
    )
    .bind(account_id)
    .bind("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    .bind("2026-01-01T00:11:00Z")
    .bind("2026-01-01T00:11:00Z")
    .execute(&pool)
    .await;
    assert!(
        text_content_hash.is_err(),
        "content_sha256 must be stored as a 32-byte BLOB, not 32 chars of text"
    );

    let bad_mailbox_json = sqlx::query(
        "INSERT INTO provider_message_mappings \
         (provider_account_id, provider_message_id, jmap_mailbox_ids_json, import_status, created_at, updated_at) \
         VALUES (?, 'gmail-msg-bad-mailboxes', ?, 'pending', ?, ?)",
    )
    .bind(account_id)
    .bind("not-json")
    .bind("2026-01-01T00:11:00Z")
    .bind("2026-01-01T00:11:00Z")
    .execute(&pool)
    .await;
    assert!(
        bad_mailbox_json.is_err(),
        "jmap_mailbox_ids_json must be valid JSON when present"
    );

    sqlx::query(
        "INSERT INTO provider_sync_events \
         (provider_account_id, user_id, operation_kind, event_type, provider_message_id, result_status, metadata_json, created_at) \
         VALUES (?, ?, 'message_import', 'message_imported', 'gmail-msg-1', 'succeeded', ?, ?)",
    )
    .bind(account_id)
    .bind(user_id)
    .bind(r#"{"jmapEmailId":"jmap-email-1"}"#)
    .bind("2026-01-01T00:10:01Z")
    .execute(&pool)
    .await
    .expect("sync event insert");

    let bad_metadata_json = sqlx::query(
        "INSERT INTO provider_sync_events \
         (provider_account_id, user_id, operation_kind, event_type, provider_message_id, result_status, metadata_json, created_at) \
         VALUES (?, ?, 'message_import', 'message_imported', 'gmail-msg-bad-json', 'succeeded', ?, ?)",
    )
    .bind(account_id)
    .bind(user_id)
    .bind("not-json")
    .bind("2026-01-01T00:10:02Z")
    .execute(&pool)
    .await;
    assert!(
        bad_metadata_json.is_err(),
        "provider audit metadata_json must be valid JSON when present"
    );

    sqlx::query("DELETE FROM provider_accounts WHERE id = ?")
        .bind(account_id)
        .execute(&pool)
        .await
        .expect("provider account delete cascades");

    let mappings: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM provider_message_mappings WHERE provider_account_id = ?",
    )
    .bind(account_id)
    .fetch_one(&pool)
    .await
    .expect("mapping count");
    let events: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM provider_sync_events WHERE provider_account_id = ?",
    )
    .bind(account_id)
    .fetch_one(&pool)
    .await
    .expect("event count");
    assert_eq!(mappings, 0, "mappings must cascade with provider account");
    assert_eq!(events, 0, "sync events must cascade with provider account");
}

#[tokio::test]
async fn labels_schema_supports_flat_thread_labels_and_cascades() {
    let (pool, _guard) = setup().await;

    sqlx::query("INSERT INTO users (email, jmap_account_id, created_at) VALUES (?, ?, ?)")
        .bind("labels@example.com")
        .bind("acct-labels")
        .bind("2026-01-01T00:00:00Z")
        .execute(&pool)
        .await
        .expect("user insert");

    let user_id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE email = ?")
        .bind("labels@example.com")
        .fetch_one(&pool)
        .await
        .expect("fetch user id");

    sqlx::query(
        "INSERT INTO labels \
         (user_id, name, normalized_name, source, provider_kind, provider_label_id, color, created_at, updated_at) \
         VALUES (?, ?, ?, 'gmail', 'gmail', ?, ?, ?, ?)",
    )
    .bind(user_id)
    .bind("Work/Receipts")
    .bind("work/receipts")
    .bind("Label_123")
    .bind("#3b82f6")
    .bind("2026-01-01T00:00:00Z")
    .bind("2026-01-01T00:00:00Z")
    .execute(&pool)
    .await
    .expect("flat nested label insert");

    let label_id: i64 = sqlx::query_scalar("SELECT id FROM labels WHERE user_id = ?")
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .expect("fetch label id");

    let parent_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM labels WHERE user_id = ? AND normalized_name = 'work'",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .expect("parent label count");
    assert_eq!(
        parent_count, 0,
        "Work/Receipts must be stored as one flat concrete label, not imply Work"
    );

    sqlx::query("INSERT INTO thread_labels (user_id, thread_id, label_id, created_at) VALUES (?, ?, ?, ?)")
        .bind(user_id)
        .bind("thread-1")
        .bind(label_id)
        .bind("2026-01-01T00:00:01Z")
        .execute(&pool)
        .await
        .expect("thread label insert");

    let duplicate_assignment = sqlx::query(
        "INSERT INTO thread_labels (user_id, thread_id, label_id, created_at) VALUES (?, ?, ?, ?)",
    )
    .bind(user_id)
    .bind("thread-1")
    .bind(label_id)
    .bind("2026-01-01T00:00:02Z")
    .execute(&pool)
    .await;
    assert!(
        duplicate_assignment.is_err(),
        "thread label assignments must be unique per user/thread/label"
    );

    sqlx::query("DELETE FROM labels WHERE id = ?")
        .bind(label_id)
        .execute(&pool)
        .await
        .expect("label delete cascades");

    let assignment_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM thread_labels WHERE label_id = ?")
            .bind(label_id)
            .fetch_one(&pool)
            .await
            .expect("assignment count");
    assert_eq!(
        assignment_count, 0,
        "deleting a label must delete thread label assignments"
    );
}

#[tokio::test]
async fn labels_schema_enforces_uniqueness_and_provider_identity() {
    let (pool, _guard) = setup().await;

    for (email, acct) in [
        ("label-unique-a@example.com", "acct-label-unique-a"),
        ("label-unique-b@example.com", "acct-label-unique-b"),
    ] {
        sqlx::query("INSERT INTO users (email, jmap_account_id, created_at) VALUES (?, ?, ?)")
            .bind(email)
            .bind(acct)
            .bind("2026-01-01T00:00:00Z")
            .execute(&pool)
            .await
            .expect("user insert");
    }

    let user_a: i64 = sqlx::query_scalar("SELECT id FROM users WHERE email = ?")
        .bind("label-unique-a@example.com")
        .fetch_one(&pool)
        .await
        .expect("fetch user a");
    let user_b: i64 = sqlx::query_scalar("SELECT id FROM users WHERE email = ?")
        .bind("label-unique-b@example.com")
        .fetch_one(&pool)
        .await
        .expect("fetch user b");

    sqlx::query(
        "INSERT INTO labels (user_id, name, normalized_name, source, created_at, updated_at) \
         VALUES (?, 'Work/Receipts', 'work/receipts', 'manual', ?, ?)",
    )
    .bind(user_a)
    .bind("2026-01-01T00:00:00Z")
    .bind("2026-01-01T00:00:00Z")
    .execute(&pool)
    .await
    .expect("user a label insert");

    let duplicate_normalized = sqlx::query(
        "INSERT INTO labels (user_id, name, normalized_name, source, created_at, updated_at) \
         VALUES (?, 'work/receipts', 'work/receipts', 'manual', ?, ?)",
    )
    .bind(user_a)
    .bind("2026-01-01T00:00:01Z")
    .bind("2026-01-01T00:00:01Z")
    .execute(&pool)
    .await;
    assert!(
        duplicate_normalized.is_err(),
        "normalized label names must be unique per user"
    );

    sqlx::query(
        "INSERT INTO labels (user_id, name, normalized_name, source, created_at, updated_at) \
         VALUES (?, 'Work/Receipts', 'work/receipts', 'manual', ?, ?)",
    )
    .bind(user_b)
    .bind("2026-01-01T00:00:00Z")
    .bind("2026-01-01T00:00:00Z")
    .execute(&pool)
    .await
    .expect("same normalized name is allowed for a different user");

    sqlx::query(
        "INSERT INTO labels \
         (user_id, name, normalized_name, source, provider_kind, provider_label_id, created_at, updated_at) \
         VALUES (?, 'Gmail Label', 'gmail label', 'gmail', 'gmail', 'Label_1', ?, ?)",
    )
    .bind(user_a)
    .bind("2026-01-01T00:00:02Z")
    .bind("2026-01-01T00:00:02Z")
    .execute(&pool)
    .await
    .expect("gmail label insert");

    let duplicate_provider = sqlx::query(
        "INSERT INTO labels \
         (user_id, name, normalized_name, source, provider_kind, provider_label_id, created_at, updated_at) \
         VALUES (?, 'Gmail Label Renamed', 'gmail label renamed', 'gmail', 'gmail', 'Label_1', ?, ?)",
    )
    .bind(user_a)
    .bind("2026-01-01T00:00:03Z")
    .bind("2026-01-01T00:00:03Z")
    .execute(&pool)
    .await;
    assert!(
        duplicate_provider.is_err(),
        "provider label ids must be unique per user/provider when present"
    );
}

#[tokio::test]
async fn labels_schema_rejects_invalid_shape_values() {
    let (pool, _guard) = setup().await;

    sqlx::query("INSERT INTO users (email, jmap_account_id, created_at) VALUES (?, ?, ?)")
        .bind("label-invalid@example.com")
        .bind("acct-label-invalid")
        .bind("2026-01-01T00:00:00Z")
        .execute(&pool)
        .await
        .expect("user insert");

    let user_id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE email = ?")
        .bind("label-invalid@example.com")
        .fetch_one(&pool)
        .await
        .expect("fetch user id");

    let empty_name = sqlx::query(
        "INSERT INTO labels (user_id, name, normalized_name, source, created_at, updated_at) \
         VALUES (?, '', 'empty', 'manual', ?, ?)",
    )
    .bind(user_id)
    .bind("2026-01-01T00:00:00Z")
    .bind("2026-01-01T00:00:00Z")
    .execute(&pool)
    .await;
    assert!(empty_name.is_err(), "empty label names must be rejected");

    let empty_segment = sqlx::query(
        "INSERT INTO labels (user_id, name, normalized_name, source, created_at, updated_at) \
         VALUES (?, 'Work//Receipts', 'work//receipts', 'manual', ?, ?)",
    )
    .bind(user_id)
    .bind("2026-01-01T00:00:00Z")
    .bind("2026-01-01T00:00:00Z")
    .execute(&pool)
    .await;
    assert!(
        empty_segment.is_err(),
        "empty path segments must be rejected"
    );

    let bad_source = sqlx::query(
        "INSERT INTO labels (user_id, name, normalized_name, source, created_at, updated_at) \
         VALUES (?, 'Work', 'work', 'imap', ?, ?)",
    )
    .bind(user_id)
    .bind("2026-01-01T00:00:00Z")
    .bind("2026-01-01T00:00:00Z")
    .execute(&pool)
    .await;
    assert!(bad_source.is_err(), "label source must be constrained");

    let partial_provider_identity = sqlx::query(
        "INSERT INTO labels \
         (user_id, name, normalized_name, source, provider_kind, created_at, updated_at) \
         VALUES (?, 'Gmail', 'gmail', 'gmail', 'gmail', ?, ?)",
    )
    .bind(user_id)
    .bind("2026-01-01T00:00:00Z")
    .bind("2026-01-01T00:00:00Z")
    .execute(&pool)
    .await;
    assert!(
        partial_provider_identity.is_err(),
        "provider_kind and provider_label_id must be present or absent together"
    );
}

#[tokio::test]
async fn speakeasy_schema_is_per_user_secret_with_rotation_metadata() {
    let (pool, _guard) = setup().await;

    sqlx::query("INSERT INTO users (email, jmap_account_id, created_at) VALUES (?, ?, ?)")
        .bind("speakeasy-schema@example.com")
        .bind("acct-speakeasy-schema")
        .bind("2026-01-01T00:00:00Z")
        .execute(&pool)
        .await
        .expect("user insert");

    let user_id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE email = ?")
        .bind("speakeasy-schema@example.com")
        .fetch_one(&pool)
        .await
        .expect("fetch user id");

    sqlx::query(
        "INSERT INTO speakeasy_passphrases \
         (user_id, passphrase, period, rotates_at, generated_at, updated_at) \
         VALUES (?, ?, '2026-05', ?, ?, ?)",
    )
    .bind(user_id)
    .bind("amber-basil-coral-delta")
    .bind("2026-06-01T00:00:00Z")
    .bind("2026-05-27T12:00:00Z")
    .bind("2026-05-27T12:00:00Z")
    .execute(&pool)
    .await
    .expect("speakeasy insert");

    let duplicate = sqlx::query(
        "INSERT INTO speakeasy_passphrases \
         (user_id, passphrase, period, rotates_at, generated_at, updated_at) \
         VALUES (?, ?, '2026-05', ?, ?, ?)",
    )
    .bind(user_id)
    .bind("copper-ember-forest-harbor")
    .bind("2026-06-01T00:00:00Z")
    .bind("2026-05-27T12:01:00Z")
    .bind("2026-05-27T12:01:00Z")
    .execute(&pool)
    .await;
    assert!(duplicate.is_err(), "only one current phrase row per user");

    let bad_period = sqlx::query(
        "INSERT INTO speakeasy_passphrases \
         (user_id, passphrase, period, rotates_at, generated_at, updated_at) \
         VALUES (999, ?, 'May 2026', ?, ?, ?)",
    )
    .bind("juniper-lagoon-maple-olive")
    .bind("2026-06-01T00:00:00Z")
    .bind("2026-05-27T12:00:00Z")
    .bind("2026-05-27T12:00:00Z")
    .execute(&pool)
    .await;
    assert!(bad_period.is_err(), "period must be normalized YYYY-MM");

    sqlx::query("DELETE FROM users WHERE id = ?")
        .bind(user_id)
        .execute(&pool)
        .await
        .expect("delete user");
    let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM speakeasy_passphrases")
        .fetch_one(&pool)
        .await
        .expect("count speakeasy rows");
    assert_eq!(remaining, 0, "speakeasy rows cascade with user deletion");
}

#[tokio::test]
async fn foreign_keys_are_enforced() {
    let (pool, _guard) = setup().await;

    // No user with id=999; sessions.user_id FK must reject this.
    let res = sqlx::query(
        "INSERT INTO sessions (id, user_id, jmap_token_enc, expires_at, created_at, last_used_at) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind("sess-1")
    .bind(999_i64)
    .bind(vec![0u8; 16])
    .bind("2026-01-01T00:00:00Z")
    .bind("2026-01-01T00:00:00Z")
    .bind("2026-01-01T00:00:00Z")
    .execute(&pool)
    .await;

    assert!(res.is_err(), "FK on sessions.user_id must be enforced");
}
