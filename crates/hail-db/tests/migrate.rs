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
