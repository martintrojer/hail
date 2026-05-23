//! Screener view + decision endpoints.
//!
//! `GET /api/views/screener` is backed by the sidecar `screener_rules`
//! table. It deliberately returns `latest_preview: null` for now: once the
//! API has a live JMAP integration test harness, enrich each pending sender
//! via JMAP `Email/query` + `Email/get` against the hail-owned Screener
//! mailbox so `message_count` and previews reflect actual pending messages.
//!
//! `POST /api/screener/decisions` writes the user's sender decision to the
//! same table. Applying that decision to historical messages is injected via
//! [`ScreenerBackfill`] so tests can assert calls without a live JMAP server;
//! production is an explicit safe no-op until the JMAP backfill worker lands.

use std::sync::Arc;

use async_trait::async_trait;
use axum::extract::{Extension, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing::post};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::audit;
use crate::middleware::auth::AuthUser;
use crate::routes::undo::{UndoToken, create_undo_action};
use crate::state::AppState;

const DEFAULT_APPROVAL_CLASSIFICATION: Classification = Classification::Imbox;

/// Dependency-injection seam for applying a screener decision to existing
/// mail from the sender. Production is currently [`NoopScreenerBackfill`]
/// because the endpoint must be safe without a live JMAP test server.
#[async_trait]
pub trait ScreenerBackfill: Send + Sync + 'static {
    async fn apply(
        &self,
        state: &AppState,
        user: &AuthUser,
        sender: &str,
        decision: ScreenerDecision,
        classify_as: Option<Classification>,
    ) -> Result<(), ScreenerBackfillError>;
}

/// Production backfill implementation. Explicitly no-ops for v1 endpoint
/// bring-up; a future JMAP-backed implementation should move/keyword old
/// Screener messages in the same semantic way as the worker's routing path.
pub struct NoopScreenerBackfill;

#[async_trait]
impl ScreenerBackfill for NoopScreenerBackfill {
    async fn apply(
        &self,
        _state: &AppState,
        _user: &AuthUser,
        _sender: &str,
        _decision: ScreenerDecision,
        _classify_as: Option<Classification>,
    ) -> Result<(), ScreenerBackfillError> {
        tracing::debug!(
            "screener history backfill requested; production implementation is TODO no-op"
        );
        Ok(())
    }
}

/// Opaque backfill failure. Details stay in server logs only.
#[derive(Debug)]
pub struct ScreenerBackfillError(pub String);

/// Build protected screener routes.
pub fn router() -> Router<AppState> {
    router_with_backfill(Arc::new(NoopScreenerBackfill))
}

/// Test/helper router that injects a fake backfill implementation.
pub fn router_with_backfill<B>(backfill: Arc<B>) -> Router<AppState>
where
    B: ScreenerBackfill,
{
    Router::new()
        .route("/api/views/screener", axum::routing::get(get_screener))
        .route("/api/screener/decisions", post(post_decision::<B>))
        .layer(Extension(backfill))
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

#[derive(Debug, Deserialize)]
struct DecisionRequest {
    sender: String,
    decision: String,
    classify_as: Option<String>,
    apply_to_history: bool,
}

#[derive(Debug, Serialize)]
struct DecisionResponse {
    sender: String,
    decision: &'static str,
    classify_as: Option<Classification>,
    undo: Option<UndoToken>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenerDecision {
    Approve,
    Deny,
}

impl ScreenerDecision {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "approve" => Some(Self::Approve),
            "deny" => Some(Self::Deny),
            _ => None,
        }
    }

    const fn response_value(self) -> &'static str {
        match self {
            Self::Approve => "approve",
            Self::Deny => "deny",
        }
    }

    const fn db_value(self) -> &'static str {
        match self {
            Self::Approve => "allow",
            Self::Deny => "deny",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Classification {
    Imbox,
    Feed,
    Papertrail,
}

impl Classification {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "imbox" => Some(Self::Imbox),
            "feed" => Some(Self::Feed),
            "papertrail" => Some(Self::Papertrail),
            _ => None,
        }
    }

    const fn db_value(self) -> &'static str {
        match self {
            Self::Imbox => "imbox",
            Self::Feed => "feed",
            Self::Papertrail => "papertrail",
        }
    }
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

async fn post_decision<B>(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(backfill): Extension<Arc<B>>,
    Json(body): Json<DecisionRequest>,
) -> Response
where
    B: ScreenerBackfill,
{
    let sender = normalize_sender(&body.sender);
    if sender.is_empty() {
        return bad_request("invalid_sender");
    }

    let Some(decision) = ScreenerDecision::parse(&body.decision) else {
        return bad_request("invalid_decision");
    };

    // Product default: approving without `classify_as` routes future mail to
    // Imbox. Clients may still send an explicit classification.
    let classify_as = match decision {
        ScreenerDecision::Approve => match body.classify_as.as_deref() {
            Some(raw) => match Classification::parse(raw) {
                Some(classification) => classification,
                None => return bad_request("invalid_classify_as"),
            },
            None => DEFAULT_APPROVAL_CLASSIFICATION,
        },
        ScreenerDecision::Deny => match body.classify_as.as_deref() {
            Some(raw) if Classification::parse(raw).is_none() => {
                return bad_request("invalid_classify_as");
            }
            _ => DEFAULT_APPROVAL_CLASSIFICATION,
        },
    };

    let response_classify_as = match decision {
        ScreenerDecision::Approve => Some(classify_as),
        ScreenerDecision::Deny => None,
    };

    let previous_rule = match sqlx::query_as::<_, ScreenerRuleSnapshot>(
        "SELECT decision, classify_as, decided_at, first_seen_at \
         FROM screener_rules WHERE user_id = ?1 AND sender_address = ?2",
    )
    .bind(user.id)
    .bind(&sender)
    .fetch_optional(&state.db)
    .await
    {
        Ok(rule) => rule,
        Err(err) => {
            tracing::error!(user_id = user.id, sender = %sender, error = %err, "screener previous rule lookup failed");
            return internal();
        }
    };

    let now = Utc::now();
    let classify_as_db = response_classify_as.map(Classification::db_value);
    if let Err(err) = sqlx::query(
        "INSERT INTO screener_rules \
         (user_id, sender_address, decision, classify_as, decided_at, first_seen_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?5) \
         ON CONFLICT(user_id, sender_address) DO UPDATE SET \
           decision = excluded.decision, \
           classify_as = excluded.classify_as, \
           decided_at = excluded.decided_at",
    )
    .bind(user.id)
    .bind(&sender)
    .bind(decision.db_value())
    .bind(classify_as_db)
    .bind(now)
    .execute(&state.db)
    .await
    {
        tracing::error!(user_id = user.id, sender = %sender, error = %err, "screener decision upsert failed");
        return internal();
    }

    if body.apply_to_history {
        if let Err(err) = backfill
            .apply(&state, &user, &sender, decision, response_classify_as)
            .await
        {
            tracing::error!(user_id = user.id, sender = %sender, error = %err.0, "screener history backfill failed");
            return internal();
        }
    }

    if let Err(err) = audit::record(
        &state.db,
        user.id,
        "screener.decision",
        &serde_json::json!({
            "sender": &sender,
            "decision": decision.response_value(),
            "classify_as": response_classify_as,
            "apply_to_history": body.apply_to_history,
        }),
    )
    .await
    {
        tracing::warn!(user_id = user.id, sender = %sender, error = %err, "audit log write failed");
    }

    let undo = match create_undo_action(
        &state,
        user.id,
        "screener.decision",
        serde_json::json!({
            "sender": &sender,
            "previous_rule": previous_rule,
        }),
    )
    .await
    {
        Ok(undo) => Some(undo),
        Err(err) => {
            tracing::warn!(user_id = user.id, sender = %sender, error = %err, "undo action create failed");
            None
        }
    };

    Json(DecisionResponse {
        sender,
        decision: decision.response_value(),
        classify_as: response_classify_as,
        undo,
    })
    .into_response()
}

#[derive(Debug, Serialize, sqlx::FromRow)]
struct ScreenerRuleSnapshot {
    decision: String,
    classify_as: Option<String>,
    decided_at: Option<DateTime<Utc>>,
    first_seen_at: DateTime<Utc>,
}

fn normalize_sender(sender: &str) -> String {
    sender.trim().to_ascii_lowercase()
}

fn bad_request(error: &'static str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        [(header::CONTENT_TYPE, "application/json")],
        format!(r#"{{"error":"{error}"}}"#),
    )
        .into_response()
}

fn internal() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"error":"internal"}"#,
    )
        .into_response()
}
