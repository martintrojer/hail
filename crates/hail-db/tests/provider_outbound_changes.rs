use sqlx::Row;

async fn setup_db() -> sqlx::SqlitePool {
    let db = hail_db::connect("sqlite::memory:").await.expect("connect");
    hail_db::migrate(&db).await.expect("migrate");
    db
}

async fn seed_mapping(db: &sqlx::SqlitePool, bidi_enabled: bool) {
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query("INSERT INTO users (id, email, jmap_account_id, created_at) VALUES (1, 'u@example.com', 'acct', ?1)")
        .bind(&now)
        .execute(db)
        .await
        .expect("user");
    sqlx::query(
        "INSERT INTO provider_accounts (id, user_id, jmap_account_id, provider_kind, provider_account_id, provider_email, display_email, granted_scopes_json, refresh_token_enc, refresh_token_key_id, sync_status, bidirectional_sync_enabled, created_at, updated_at) VALUES (10, 1, 'acct', 'gmail', 'u@example.com', 'u@example.com', 'u@example.com', '[]', zeroblob(29), 'server_key:v1', 'active', ?1, ?2, ?2)",
    )
    .bind(if bidi_enabled { 1_i64 } else { 0_i64 })
    .bind(&now)
    .execute(db)
    .await
    .expect("provider account");
    sqlx::query(
        "INSERT INTO provider_message_mappings (provider_account_id, provider_message_id, provider_thread_id, jmap_email_id, jmap_thread_id, import_status, created_at, updated_at) VALUES (10, 'gmail-1', 'gthread-1', 'email-1', 'thread-1', 'imported', ?1, ?1)",
    )
    .bind(&now)
    .execute(db)
    .await
    .expect("mapping");
}

#[tokio::test]
async fn marking_seen_with_bidi_enabled_inserts_read_outbound_row() {
    let db = setup_db().await;
    seed_mapping(&db, true).await;

    let inserted = hail_db::provider_outbound_changes::enqueue_read_state_if_bidi_enabled(
        &db, 1, "email-1", true,
    )
    .await
    .expect("enqueue");

    assert!(inserted);
    let row = sqlx::query("SELECT change_type, payload_json FROM provider_outbound_changes")
        .fetch_one(&db)
        .await
        .expect("row");
    assert_eq!(row.get::<String, _>("change_type"), "read");
    assert_eq!(row.get::<String, _>("payload_json"), "{}");
}

#[tokio::test]
async fn marking_seen_with_bidi_disabled_is_noop() {
    let db = setup_db().await;
    seed_mapping(&db, false).await;

    let inserted = hail_db::provider_outbound_changes::enqueue_read_state_if_bidi_enabled(
        &db, 1, "email-1", true,
    )
    .await
    .expect("enqueue");

    assert!(!inserted);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM provider_outbound_changes")
        .fetch_one(&db)
        .await
        .expect("count");
    assert_eq!(count, 0);
}
