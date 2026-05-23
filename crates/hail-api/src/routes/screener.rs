//! Screener view endpoint.
//!
//! `GET /api/views/screener` is backed by the sidecar `screener_rules`
//! table. It deliberately returns `latest_preview: null` for now: once the
//! API has a live JMAP integration test harness, enrich each pending sender
//! via JMAP `Email/query` + `Email/get` against the hail-owned Screener
//! mailbox so `message_count` and previews reflect actual pending messages.

use axum::extract::{Extension, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::middleware::auth::AuthUser;
use crate::state::AppState;

/// Build protected screener routes.
pub fn router() -> Router<AppState> {
    Router::new().route("/api/views/screener", axum::routing::get(get_screener))
}

#[derive(Debug, Serialize)]
struct ScreenerViewResponse {
    senders: Vec<ScreenerSender>,
}

#[derive(Debug, Serialize)]
struct ScreenerSender {
    sender: String,
    first_seen_at: DateTime<Utc>,
    message_count: i64,
    latest_preview: Option<serde_json::Value>,
}

async fn get_screener(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Response {
    let rows = match sqlx::query_as::<_, (String, DateTime<Utc>)>(
        "SELECT sender_address, first_seen_at \
         FROM screener_rules \
         WHERE user_id = ?1 AND decision = 'pending' \
         ORDER BY first_seen_at DESC",
    )
    .bind(user.id)
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => rows,
        Err(err) => {
            tracing::error!(user_id = user.id, error = %err, "screener view lookup failed");
            return internal();
        }
    };

    let senders = rows
        .into_iter()
        .map(|(sender, first_seen_at)| ScreenerSender {
            sender,
            first_seen_at,
            // TODO(JMAP Email/query): count actual pending messages per sender in
            // the Screener mailbox. The sidecar rule table has one row per sender,
            // so `1` is the safest non-zero placeholder for a pending sender.
            message_count: 1,
            latest_preview: None,
        })
        .collect();

    Json(ScreenerViewResponse { senders }).into_response()
}

fn internal() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"error":"internal"}"#,
    )
        .into_response()
}
