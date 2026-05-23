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
];

/// Indices explicitly declared in §6.2. Partial indices count as well.
const EXPECTED_INDICES: &[&str] = &[
    "idx_sessions_user",
    "idx_stack_order",
    "idx_bubble_ups_pending",
    "idx_scheduled_sends_due",
    "idx_undo_actions_user_live",
];

/// Build a fresh DB URL backed by a unique temp file. We deliberately avoid
/// `sqlite::memory:` because a multi-connection pool gives each connection
/// its own private in-memory DB, which makes migrations invisible to later
/// queries. A scratch file is also closer to the production code path
/// (WAL + sync NORMAL + foreign_keys).
fn fresh_db_url() -> (String, std::path::PathBuf) {
    let mut path = std::env::temp_dir();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let pid = std::process::id();
    path.push(format!("hail-db-test-{pid}-{nanos}.sqlite"));
    let url = format!("sqlite://{}", path.display());
    (url, path)
}

struct TempDb(std::path::PathBuf);

impl Drop for TempDb {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
        let _ = std::fs::remove_file(self.0.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(self.0.with_extension("sqlite-shm"));
    }
}

async fn setup() -> (sqlx::SqlitePool, TempDb) {
    let (url, path) = fresh_db_url();
    let guard = TempDb(path);
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
    assert!(!names.contains("email_notes"), "v1.1 table leaked into baseline");
    assert!(!names.contains("clips"), "v1.1 table leaked into baseline");
}

#[tokio::test]
async fn users_email_is_unique() {
    let (pool, _guard) = setup().await;

    sqlx::query(
        "INSERT INTO users (email, jmap_account_id, created_at) VALUES (?, ?, ?)",
    )
    .bind("alice@example.com")
    .bind("acct-1")
    .bind("2026-01-01T00:00:00Z")
    .execute(&pool)
    .await
    .expect("first insert");

    let dup = sqlx::query(
        "INSERT INTO users (email, jmap_account_id, created_at) VALUES (?, ?, ?)",
    )
    .bind("alice@example.com")
    .bind("acct-2")
    .bind("2026-01-02T00:00:00Z")
    .execute(&pool)
    .await;

    assert!(dup.is_err(), "duplicate email must violate UNIQUE constraint");
}

#[tokio::test]
async fn screener_rules_decision_check_enforced() {
    let (pool, _guard) = setup().await;

    sqlx::query(
        "INSERT INTO users (email, jmap_account_id, created_at) VALUES (?, ?, ?)",
    )
    .bind("bob@example.com")
    .bind("acct-bob")
    .bind("2026-01-01T00:00:00Z")
    .execute(&pool)
    .await
    .expect("user insert");

    let user_id: i64 =
        sqlx::query_scalar("SELECT id FROM users WHERE email = ?")
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

    assert!(bad.is_err(), "CHECK constraint on decision must reject `maybe`");
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
