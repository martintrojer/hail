//! Durable app-event outbox shared by `hail-worker` publishers and
//! `hail-api` WebSocket broadcasting.
//!
//! This is intentionally a tiny SQLite-backed bridge for v1. Workers append
//! coarse invalidation events after durable state/JMAP transitions; API
//! processes poll for rows newer than their in-memory cursor and fan them out to
//! process-local WebSocket subscribers. Rows are not deleted in v1, so an API
//! restart resumes from the current max id instead of replaying historical
//! events to already-stale browser tabs. The browser treats these as hints and
//! refetches current state, so duplicate delivery is safe.

use sqlx::{Row, SqlitePool};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredAppEvent {
    pub id: i64,
    pub user_id: Option<i64>,
    pub event_type: String,
    pub payload_json: String,
}

/// Append an app event to the durable outbox.
pub async fn insert_app_event(
    db: &SqlitePool,
    user_id: Option<i64>,
    event_type: &str,
    payload_json: &str,
) -> Result<i64, sqlx::Error> {
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query_scalar(
        "INSERT INTO app_events (user_id, event_type, payload_json, created_at) \
         VALUES (?1, ?2, ?3, ?4) RETURNING id",
    )
    .bind(user_id)
    .bind(event_type)
    .bind(payload_json)
    .bind(now)
    .fetch_one(db)
    .await
}

/// Return the newest event id currently present in the outbox.
pub async fn latest_app_event_id(db: &SqlitePool) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COALESCE(MAX(id), 0) FROM app_events")
        .fetch_one(db)
        .await
}

/// Fetch events strictly newer than `after_id`, ordered for broadcast.
pub async fn fetch_app_events_after(
    db: &SqlitePool,
    after_id: i64,
    limit: i64,
) -> Result<Vec<StoredAppEvent>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT id, user_id, event_type, payload_json \
         FROM app_events \
         WHERE id > ?1 \
         ORDER BY id ASC \
         LIMIT ?2",
    )
    .bind(after_id)
    .bind(limit)
    .fetch_all(db)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| StoredAppEvent {
            id: row.get("id"),
            user_id: row.get("user_id"),
            event_type: row.get("event_type"),
            payload_json: row.get("payload_json"),
        })
        .collect())
}
