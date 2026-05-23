//! Append-only audit logging helpers.

use chrono::Utc;
use serde::Serialize;
use sqlx::SqlitePool;

/// Append an audit event for a user action.
///
/// Payload serialization failures are returned before any database write. The
/// database stores JSON as text so schema changes are not needed for new audit
/// payload shapes.
pub async fn record<P>(
    db: &SqlitePool,
    user_id: i64,
    action: &str,
    payload: &P,
) -> Result<(), AuditError>
where
    P: Serialize + ?Sized,
{
    let payload_json = serde_json::to_string(payload)?;
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO audit_log (user_id, action, payload_json, created_at) VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(user_id)
    .bind(action)
    .bind(payload_json)
    .bind(now)
    .execute(db)
    .await?;
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum AuditError {
    #[error("serialize audit payload: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("insert audit row: {0}")]
    Database(#[from] sqlx::Error),
}

#[cfg(test)]
mod tests {
    use super::record;
    use hail_db::{connect, migrate};
    use serde_json::json;

    #[tokio::test]
    async fn record_inserts_row() {
        let url = format!(
            "sqlite:file:hail_audit_test_{}?mode=memory&cache=shared",
            uuid_like()
        );
        let db = connect(&url).await.expect("open sqlite");
        migrate(&db).await.expect("migrate");
        let user_id: i64 = sqlx::query_scalar(
            "INSERT INTO users (email, jmap_account_id, created_at) \
             VALUES ('audit@example.org', 'account-audit', datetime('now')) RETURNING id",
        )
        .fetch_one(&db)
        .await
        .expect("insert user");

        record(
            &db,
            user_id,
            "unit.test",
            &json!({ "answer": 42, "ok": true }),
        )
        .await
        .expect("record audit");

        let row: (i64, String, String, Option<String>) =
            sqlx::query_as("SELECT user_id, action, payload_json, created_at FROM audit_log")
                .fetch_one(&db)
                .await
                .expect("audit row");
        assert_eq!(row.0, user_id);
        assert_eq!(row.1, "unit.test");
        assert_eq!(row.2, r#"{"answer":42,"ok":true}"#);
        assert!(row.3.is_some());
    }

    fn uuid_like() -> String {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        format!(
            "{}_{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        )
    }
}
