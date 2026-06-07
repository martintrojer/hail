//! Screener view + decision endpoints.
//!
//! `GET /api/views/screener` starts from the sidecar `screener_rules`
//! table, then best-effort enriches each pending sender from CachedMail. If mail enrichment is unavailable
//! (expired token, backend down, sparse cache), the endpoint keeps the
//! sidecar-only fallback shape so the Screener UI remains usable.
//!
//! `POST /api/screener/decisions` writes the user's sender decision to the
//! same table. Applying that decision to historical messages is injected via
//! [`ScreenerBackfill`] so tests can assert calls without a live mail backend;
//! production uses [`CacheScreenerBackfill`] to move/keyword existing Screener
//! messages as soon as the sender is approved or denied.

use std::sync::Arc;

use async_trait::async_trait;
use axum::extract::{Extension, Path, Query, State, rejection::JsonRejection};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
pub use hail_core::screener::Classification;
use hail_core::screener::normalize_sender;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use utoipa_axum::router::{OpenApiRouter, UtoipaMethodRouterExt};
use utoipa_axum::routes;

use crate::audit;
use crate::middleware::auth::AuthUser;
use crate::routes::response::{bad_request, internal};
use crate::routes::undo::{NewUndoAction, UndoToken, create_undo_action};
use crate::state::AppState;

/// OpenAPI tag for screener sender-review endpoints.
pub const TAG: &str = "screener";

const DEFAULT_APPROVAL_CLASSIFICATION: Classification = Classification::Imbox;
const SCREENER_LIGHT_EMAIL_GET_CHUNK_SIZE: usize = 200;
const DEFAULT_SCREENER_LIMIT: usize = 50;
const MAX_SCREENER_LIMIT: usize = 100;

/// Dependency-injection seam for applying a screener decision to existing
/// mail from the sender. Production uses [`CacheScreenerBackfill`]; tests can
/// inject fakes with [`router_with_backfill`].
#[async_trait]
pub trait ScreenerBackfill: Send + Sync + 'static {
    async fn apply(
        &self,
        state: &AppState,
        _user: &AuthUser,
        sender: &str,
        decision: ScreenerDecision,
        classify_as: Option<Classification>,
    ) -> Result<(), ScreenerBackfillError>;

    async fn apply_undo_deny(
        &self,
        state: &AppState,
        _user: &AuthUser,
        sender: &str,
        classify_as: Classification,
    ) -> Result<(), ScreenerBackfillError> {
        self.apply(
            state,
            _user,
            sender,
            ScreenerDecision::Approve,
            Some(classify_as),
        )
        .await
    }
}

/// Production backfill implementation that applies a screener decision to
/// existing messages from the sender currently parked in the hail-owned
/// `Screener` mailbox.
pub struct CacheScreenerBackfill;

pub type JmapScreenerBackfill = CacheScreenerBackfill;

#[async_trait]
impl ScreenerBackfill for CacheScreenerBackfill {
    async fn apply(
        &self,
        state: &AppState,
        _user: &AuthUser,
        sender: &str,
        decision: ScreenerDecision,
        classify_as: Option<Classification>,
    ) -> Result<(), ScreenerBackfillError> {
        apply_cache_backfill(state, sender, decision, classify_as).await
    }

    async fn apply_undo_deny(
        &self,
        state: &AppState,
        _user: &AuthUser,
        sender: &str,
        classify_as: Classification,
    ) -> Result<(), ScreenerBackfillError> {
        apply_cache_undo_deny_backfill(state, sender, classify_as).await
    }
}

/// Opaque backfill failure. Details stay in server logs only.
#[derive(Debug)]
pub struct ScreenerBackfillError(pub String);

/// Build protected screener routes.
pub fn router() -> Router<AppState> {
    Router::from(openapi_router_with_backfill(Arc::new(CacheScreenerBackfill)))
}

/// Build the OpenAPI-tracked router for production screener endpoints.
pub fn openapi_router() -> OpenApiRouter<AppState> {
    openapi_router_with_backfill(Arc::new(CacheScreenerBackfill))
}

/// Test/helper router that injects a fake backfill implementation.
pub fn router_with_backfill<B>(backfill: Arc<B>) -> Router<AppState>
where
    B: ScreenerBackfill,
{
    Router::from(openapi_router_with_backfill(backfill))
}

fn openapi_router_with_backfill<B>(backfill: Arc<B>) -> OpenApiRouter<AppState>
where
    B: ScreenerBackfill,
{
    let backfill: Arc<dyn ScreenerBackfill> = backfill;
    OpenApiRouter::new()
        .routes(routes!(get_screener))
        .routes(routes!(get_allowed_senders))
        .routes(routes!(get_denied_senders))
        .routes(routes!(post_undo_deny).layer(Extension(backfill.clone())))
        .routes(routes!(post_decision).layer(Extension(backfill)))
}

#[derive(Debug, Serialize, ToSchema)]
struct ScreenerViewResponse {
    senders: Vec<ScreenerSender>,
    next_cursor: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
struct ScreenerViewQuery {
    cursor: Option<String>,
    limit: Option<usize>,
}

impl ScreenerViewQuery {
    fn normalized_limit(&self) -> usize {
        self.limit
            .unwrap_or(DEFAULT_SCREENER_LIMIT)
            .clamp(1, MAX_SCREENER_LIMIT)
    }

    fn decoded_cursor(&self) -> Result<Option<ScreenerCursor>, ()> {
        match self.cursor.as_deref().map(str::trim) {
            None | Some("") => Ok(None),
            Some(cursor) => ScreenerCursor::decode(cursor).map(Some),
        }
    }
}

#[derive(Debug)]
struct ScreenerCursor {
    sort_key: DateTime<Utc>,
    sender: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
struct ScreenerSender {
    sender: String,
    #[schema(value_type = String, format = DateTime)]
    first_seen_at: DateTime<Utc>,
    message_count: i64,
    latest_preview: Option<ScreenerLatestPreview>,
    emails: Vec<ScreenerEmail>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
struct ScreenerEmail {
    email_id: String,
    subject: String,
    preview: String,
    #[schema(value_type = Option<String>, format = DateTime)]
    received_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
struct ScreenerLatestPreview {
    subject: String,
    preview: String,
    from: String,
    #[schema(value_type = Option<String>, format = DateTime)]
    received_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize, ToSchema)]
struct DecisionRequest {
    sender: String,
    decision: String,
    classify_as: Option<String>,
    apply_to_history: bool,
}

#[derive(Debug, Serialize, ToSchema)]
struct AllowedSendersResponse {
    allowed: Vec<AllowedSender>,
}

#[derive(Debug, Serialize, ToSchema)]
struct AllowedSender {
    sender_address: String,
    classify_as: Classification,
    #[schema(value_type = String, format = DateTime)]
    first_seen_at: DateTime<Utc>,
    #[schema(value_type = Option<String>, format = DateTime)]
    decided_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, ToSchema)]
struct DeniedSendersResponse {
    denied: Vec<DeniedSender>,
}

#[derive(Debug, Serialize, ToSchema)]
struct DeniedSender {
    sender_address: String,
    #[schema(value_type = String, format = DateTime)]
    denied_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, ToSchema)]
struct UndoDenyRequest {
    classify_as: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
struct UndoDenyResponse {
    status: &'static str,
    classify_as: Classification,
}

#[derive(Debug, Serialize, ToSchema)]
struct DecisionResponse {
    sender: String,
    decision: &'static str,
    classify_as: Option<Classification>,
    undo: Option<UndoToken>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ToSchema)]
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

#[utoipa::path(
    get,
    path = "/api/views/screener",
    tag = TAG,
    params(ScreenerViewQuery),
    responses(
        (status = 200, description = "Pending screener senders.", body = ScreenerViewResponse),
        (status = 400, description = "Invalid cursor."),
        (status = 401, description = "Missing or invalid session."),
        (status = 500, description = "Screener lookup failed."),
    ),
)]
async fn get_screener(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Query(query): Query<ScreenerViewQuery>,
) -> Response {
    let cursor = match query.decoded_cursor() {
        Ok(cursor) => cursor,
        Err(()) => return bad_request("invalid_cursor"),
    };
    let limit = query.normalized_limit();
    let page_size = limit.saturating_add(1) as i64;

    let rows = match if let Some(cursor) = cursor {
        sqlx::query_as::<_, (String, DateTime<Utc>, DateTime<Utc>)>(
            "SELECT sender_address, first_seen_at, COALESCE(latest_pending_received_at, first_seen_at) AS sort_key \
             FROM screener_rules \
             WHERE user_id = ?1 \
               AND decision = 'pending' \
               AND (COALESCE(latest_pending_received_at, first_seen_at) < ?2 \
                    OR (COALESCE(latest_pending_received_at, first_seen_at) = ?2 AND sender_address > ?3)) \
             ORDER BY COALESCE(latest_pending_received_at, first_seen_at) DESC, sender_address ASC \
             LIMIT ?4",
        )
        .bind(user.id)
        .bind(cursor.sort_key)
        .bind(cursor.sender)
        .bind(page_size)
        .fetch_all(&state.db)
        .await
    } else {
        sqlx::query_as::<_, (String, DateTime<Utc>, DateTime<Utc>)>(
            "SELECT sender_address, first_seen_at, COALESCE(latest_pending_received_at, first_seen_at) AS sort_key \
             FROM screener_rules \
             WHERE user_id = ?1 AND decision = 'pending' \
             ORDER BY COALESCE(latest_pending_received_at, first_seen_at) DESC, sender_address ASC \
             LIMIT ?2",
        )
        .bind(user.id)
        .bind(page_size)
        .fetch_all(&state.db)
        .await
    } {
        Ok(rows) => rows,
        Err(err) => {
            tracing::error!(user_id = user.id, error = %err, "screener view lookup failed");
            return internal();
        }
    };

    let has_more = rows.len() > limit;
    let mut rows = rows;
    if has_more {
        rows.truncate(limit);
    }
    let next_cursor = if has_more {
        rows.last()
            .map(|(sender, _first_seen_at, sort_key)| ScreenerCursor {
                sort_key: *sort_key,
                sender: normalize_sender(sender),
            })
            .map(|cursor| cursor.encode())
    } else {
        None
    };

    let mut senders: Vec<ScreenerSender> = rows
        .into_iter()
        .map(|(sender, first_seen_at, _sort_key)| {
            ScreenerSender::fallback(normalize_sender(&sender), first_seen_at)
        })
        .collect();

    if let Err(err) = enrich_screener_senders(&state, &user, &mut senders).await {
        tracing::warn!(
            user_id = user.id,
            error = %err,
            "screener mail preview enrichment failed; using sidecar fallback"
        );
    }

    Json(ScreenerViewResponse {
        senders,
        next_cursor,
    })
    .into_response()
}

impl ScreenerCursor {
    fn encode(&self) -> String {
        URL_SAFE_NO_PAD.encode(format!(
            "2\n{}\n{}",
            self.sort_key.to_rfc3339(),
            self.sender
        ))
    }

    fn decode(value: &str) -> Result<Self, ()> {
        let decoded = URL_SAFE_NO_PAD.decode(value).map_err(|_| ())?;
        let decoded = String::from_utf8(decoded).map_err(|_| ())?;
        let parts = decoded.split('\n').collect::<Vec<_>>();
        let (sort_key, sender) = match parts.as_slice() {
            ["2", sort_key, sender] => (*sort_key, *sender),
            [first_seen_at, sender] => (*first_seen_at, *sender),
            _ => return Err(()),
        };
        let sort_key = DateTime::parse_from_rfc3339(sort_key)
            .map_err(|_| ())?
            .with_timezone(&Utc);
        let sender = normalize_sender(sender);
        if sender.is_empty() {
            return Err(());
        }

        Ok(Self { sort_key, sender })
    }
}

impl ScreenerSender {
    fn fallback(sender: String, first_seen_at: DateTime<Utc>) -> Self {
        Self {
            sender,
            first_seen_at,
            message_count: 1,
            latest_preview: None,
            emails: Vec::new(),
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/views/screener/allowed",
    tag = TAG,
    responses(
        (status = 200, description = "Allowed screener senders and routing classifications.", body = AllowedSendersResponse),
        (status = 401, description = "Missing or invalid session."),
        (status = 500, description = "Allowed sender lookup failed."),
    ),
)]
async fn get_allowed_senders(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Response {
    let rows = match sqlx::query_as::<_, (String, String, DateTime<Utc>, Option<DateTime<Utc>>)>(
        "SELECT sender_address, classify_as, first_seen_at, decided_at \
         FROM screener_rules \
         WHERE user_id = ?1 AND decision = 'allow' AND classify_as IS NOT NULL \
         ORDER BY COALESCE(decided_at, first_seen_at) DESC",
    )
    .bind(user.id)
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => rows,
        Err(err) => {
            tracing::error!(user_id = user.id, error = %err, "allowed screener lookup failed");
            return internal();
        }
    };

    let allowed = rows
        .into_iter()
        .filter_map(|(sender_address, classify_as, first_seen_at, decided_at)| {
            let Some(classify_as) = Classification::parse(&classify_as) else {
                tracing::warn!(
                    user_id = user.id,
                    sender = %sender_address,
                    classify_as = %classify_as,
                    "skipping allowed screener rule with invalid classification"
                );
                return None;
            };

            Some(AllowedSender {
                sender_address: normalize_sender(&sender_address),
                classify_as,
                first_seen_at,
                decided_at,
            })
        })
        .collect();

    Json(AllowedSendersResponse { allowed }).into_response()
}

#[utoipa::path(
    get,
    path = "/api/views/screener/denied",
    tag = TAG,
    responses(
        (status = 200, description = "Denied screener senders.", body = DeniedSendersResponse),
        (status = 401, description = "Missing or invalid session."),
        (status = 500, description = "Denied sender lookup failed."),
    ),
)]
async fn get_denied_senders(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Response {
    let denied = match sqlx::query_as::<_, (String, DateTime<Utc>)>(
        "SELECT sender_address, COALESCE(decided_at, first_seen_at) AS denied_at \
         FROM screener_rules \
         WHERE user_id = ?1 AND decision = 'deny' \
         ORDER BY denied_at DESC",
    )
    .bind(user.id)
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => rows
            .into_iter()
            .map(|(sender_address, denied_at)| DeniedSender {
                sender_address: normalize_sender(&sender_address),
                denied_at,
            })
            .collect(),
        Err(err) => {
            tracing::error!(user_id = user.id, error = %err, "denied screener lookup failed");
            return internal();
        }
    };

    Json(DeniedSendersResponse { denied }).into_response()
}

#[utoipa::path(
    post,
    path = "/api/screener/{address}/undo-deny",
    tag = TAG,
    params(
        ("address" = String, Path, description = "Normalized sender address to approve and route out of screened-out mail."),
    ),
    request_body(content = UndoDenyRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "Denied sender approved and routed.", body = UndoDenyResponse),
        (status = 400, description = "Invalid sender address."),
        (status = 401, description = "Missing or invalid session."),
        (status = 500, description = "Undo deny failed."),
    ),
)]
async fn post_undo_deny(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(backfill): Extension<Arc<dyn ScreenerBackfill>>,
    Path(address): Path<String>,
    body: Result<Option<Json<UndoDenyRequest>>, JsonRejection>,
) -> Response {
    let body = match body {
        Ok(body) => body.map(|Json(body)| body),
        Err(_) => return bad_request("invalid_undo_deny_body"),
    };
    let sender = normalize_sender(&address);
    if !looks_like_sender(&sender) {
        return bad_request("invalid_sender");
    }

    let classify_as = match body.and_then(|body| body.classify_as) {
        Some(raw) => match Classification::parse(&raw) {
            Some(classification) => classification,
            None => return bad_request("invalid_classify_as"),
        },
        None => DEFAULT_APPROVAL_CLASSIFICATION,
    };

    let now = Utc::now();
    if let Err(err) = backfill
        .apply_undo_deny(&state, &user, &sender, classify_as)
        .await
    {
        tracing::error!(user_id = user.id, sender = %sender, error = %err.0, "screener undo deny backfill failed");
        return internal();
    }

    if let Err(err) = sqlx::query(
        "INSERT INTO screener_rules \
         (user_id, sender_address, decision, classify_as, decided_at, first_seen_at) \
         VALUES (?1, ?2, 'allow', ?3, ?4, ?4) \
         ON CONFLICT(user_id, sender_address) DO UPDATE SET \
           decision = 'allow', \
           classify_as = excluded.classify_as, \
           decided_at = excluded.decided_at",
    )
    .bind(user.id)
    .bind(&sender)
    .bind(classify_as.db_value())
    .bind(now)
    .execute(&state.db)
    .await
    {
        tracing::error!(user_id = user.id, sender = %sender, error = %err, "screener undo deny failed");
        return internal();
    }

    if let Err(err) = hail_cache::refresh_pinned_messages(&state.db).await {
        tracing::error!(user_id = user.id, sender = %sender, error = %err, "screener undo deny pin refresh failed");
        return internal();
    }

    if let Err(err) = audit::record(
        &state.db,
        user.id,
        "screener.undo_deny",
        &serde_json::json!({ "sender": &sender, "classify_as": classify_as }),
    )
    .await
    {
        tracing::warn!(user_id = user.id, sender = %sender, error = %err, "audit log write failed");
    }

    Json(UndoDenyResponse {
        status: "approved",
        classify_as,
    })
    .into_response()
}

async fn enrich_screener_senders(
    state: &AppState,
    _user: &AuthUser,
    senders: &mut [ScreenerSender],
) -> Result<(), String> {
    if senders.is_empty() {
        return Ok(());
    }
    let sender_keys = senders.iter().map(|sender| sender.sender.clone()).collect::<Vec<_>>();
    let previews = state
        .mail
        .screener_previews(&sender_keys, SCREENER_LIGHT_EMAIL_GET_CHUNK_SIZE)
        .await
        .map_err(|err| err.to_string())?;
    for (sender, preview) in senders.iter_mut().zip(previews) {
        sender.message_count = i64::try_from(preview.message_count)
            .map_err(|_| "too many matching screener messages to render in the view".to_string())?;
        let emails = preview.emails.into_iter().map(|email| {
            let summary = ScreenerEmail {
                email_id: email.email_id.as_str().to_owned(),
                subject: email.subject.clone(),
                preview: email.preview.clone(),
                received_at: email.received_at,
            };
            let latest = ScreenerLatestPreview {
                subject: email.subject,
                preview: email.preview,
                from: email.from,
                received_at: email.received_at,
            };
            (summary, latest)
        }).collect::<Vec<_>>();
        sender.latest_preview = emails.first().map(|(_, preview)| preview.clone());
        sender.emails = emails.into_iter().map(|(summary, _)| summary).collect();
    }
    Ok(())
}

async fn apply_cache_backfill(
    state: &AppState,
    sender: &str,
    decision: ScreenerDecision,
    classify_as: Option<Classification>,
) -> Result<(), ScreenerBackfillError> {
    let decision = match decision {
        ScreenerDecision::Approve => hail_cache::ScreenerDecision::Approve,
        ScreenerDecision::Deny => hail_cache::ScreenerDecision::Deny,
    };
    let classify_as = classify_as.map(|classification| hail_backend::Keyword::new(classification.keyword()));
    state.mail.apply_screener_backfill(sender, decision, classify_as).await.map_err(backfill_error)
}

async fn apply_cache_undo_deny_backfill(
    state: &AppState,
    sender: &str,
    classify_as: Classification,
) -> Result<(), ScreenerBackfillError> {
    state.mail.undo_screener_deny(sender, hail_backend::Keyword::new(classify_as.keyword()))
        .await
        .map_err(backfill_error)
}

fn backfill_error(err: impl std::fmt::Display) -> ScreenerBackfillError {
    ScreenerBackfillError(err.to_string())
}

#[utoipa::path(
    post,
    path = "/api/screener/decisions",
    tag = TAG,
    request_body(content = DecisionRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "Screener decision saved.", body = DecisionResponse),
        (status = 400, description = "Invalid screener decision payload."),
        (status = 401, description = "Missing or invalid session."),
        (status = 500, description = "Screener decision failed."),
    ),
)]
async fn post_decision(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(backfill): Extension<Arc<dyn ScreenerBackfill>>,
    body: Result<Json<DecisionRequest>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return bad_request("invalid_decision_body");
    };
    let sender = normalize_sender(&body.sender);
    if !looks_like_sender(&sender) {
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
        ScreenerDecision::Deny => {
            if body.classify_as.is_some() {
                return bad_request("invalid_classify_as");
            }
            DEFAULT_APPROVAL_CLASSIFICATION
        }
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

    if body.apply_to_history
        && let Err(err) = backfill
            .apply(&state, &user, &sender, decision, response_classify_as)
            .await
    {
        tracing::error!(user_id = user.id, sender = %sender, error = %err.0, "screener history backfill failed");
        return internal();
    }

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

    if let Err(err) = hail_cache::refresh_pinned_messages(&state.db).await {
        tracing::error!(user_id = user.id, sender = %sender, error = %err, "screener decision pin refresh failed");
        return internal();
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
        NewUndoAction::screener_decision(&sender, previous_rule.as_ref()),
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

fn looks_like_sender(sender: &str) -> bool {
    if sender.is_empty()
        || sender.len() > 320
        || sender.contains(char::is_whitespace)
        || sender.contains(['\r', '\n'])
    {
        return false;
    }
    let Some((local, domain)) = sender.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && !domain.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !domain.contains("..")
}
