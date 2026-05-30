//! Screener view + decision endpoints.
//!
//! `GET /api/views/screener` starts from the sidecar `screener_rules`
//! table, then best-effort enriches each pending sender from JMAP messages
//! currently in the hail-owned `Screener` mailbox. If JMAP is unavailable
//! (expired token, Stalwart down, missing mailbox), the endpoint keeps the
//! sidecar-only fallback shape so the Screener UI remains usable.
//!
//! `POST /api/screener/decisions` writes the user's sender decision to the
//! same table. Applying that decision to historical messages is injected via
//! [`ScreenerBackfill`] so tests can assert calls without a live JMAP server;
//! production uses [`JmapScreenerBackfill`] to move/keyword existing Screener
//! messages as soon as the sender is approved or denied.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use async_trait::async_trait;
use axum::extract::{Extension, Path, Query, State, rejection::JsonRejection};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, TimeZone, Utc};
pub use hail_core::screener::Classification;
use hail_core::screener::normalize_sender;
use hail_jmap::{SCREENER_MAILBOX_NAME, mailbox_id_by_name};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use utoipa_axum::router::{OpenApiRouter, UtoipaMethodRouterExt};
use utoipa_axum::routes;

use crate::audit;
use crate::middleware::auth::AuthUser;
use crate::routes::jmap_helpers::{
    MAIL_VIEW_PROPERTIES, collapse_preview_whitespace, jmap_session, preview_from_email,
};
use crate::routes::response::{bad_request, internal};
use crate::routes::undo::{NewUndoAction, UndoToken, create_undo_action};
use crate::state::AppState;

/// OpenAPI tag for screener sender-review endpoints.
pub const TAG: &str = "screener";

const DEFAULT_APPROVAL_CLASSIFICATION: Classification = Classification::Imbox;
const SCREENER_RICH_EMAIL_GET_CHUNK_SIZE: usize = 20;
const SCREENER_LIGHT_EMAIL_GET_CHUNK_SIZE: usize = 200;
const DEFAULT_SCREENER_LIMIT: usize = 50;
const MAX_SCREENER_LIMIT: usize = 100;

/// Dependency-injection seam for applying a screener decision to existing
/// mail from the sender. Production uses [`JmapScreenerBackfill`]; tests can
/// inject fakes with [`router_with_backfill`].
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

    async fn apply_undo_deny(
        &self,
        state: &AppState,
        user: &AuthUser,
        sender: &str,
        classify_as: Classification,
    ) -> Result<(), ScreenerBackfillError> {
        self.apply(
            state,
            user,
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
pub struct JmapScreenerBackfill;

#[async_trait]
impl ScreenerBackfill for JmapScreenerBackfill {
    async fn apply(
        &self,
        state: &AppState,
        user: &AuthUser,
        sender: &str,
        decision: ScreenerDecision,
        classify_as: Option<Classification>,
    ) -> Result<(), ScreenerBackfillError> {
        apply_jmap_backfill(state, user, sender, decision, classify_as).await
    }

    async fn apply_undo_deny(
        &self,
        state: &AppState,
        user: &AuthUser,
        sender: &str,
        classify_as: Classification,
    ) -> Result<(), ScreenerBackfillError> {
        apply_jmap_undo_deny_backfill(state, user, sender, classify_as).await
    }
}

/// Opaque backfill failure. Details stay in server logs only.
#[derive(Debug)]
pub struct ScreenerBackfillError(pub String);

/// Build protected screener routes.
pub fn router() -> Router<AppState> {
    Router::from(openapi_router_with_backfill(Arc::new(JmapScreenerBackfill)))
}

/// Build the OpenAPI-tracked router for production screener endpoints.
pub fn openapi_router() -> OpenApiRouter<AppState> {
    openapi_router_with_backfill(Arc::new(JmapScreenerBackfill))
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
    first_seen_at: DateTime<Utc>,
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
        sqlx::query_as::<_, (String, DateTime<Utc>)>(
            "SELECT sender_address, first_seen_at \
             FROM screener_rules \
             WHERE user_id = ?1 \
               AND decision = 'pending' \
               AND (first_seen_at < ?2 OR (first_seen_at = ?2 AND sender_address > ?3)) \
             ORDER BY first_seen_at DESC, sender_address ASC \
             LIMIT ?4",
        )
        .bind(user.id)
        .bind(cursor.first_seen_at)
        .bind(cursor.sender)
        .bind(page_size)
        .fetch_all(&state.db)
        .await
    } else {
        sqlx::query_as::<_, (String, DateTime<Utc>)>(
            "SELECT sender_address, first_seen_at \
             FROM screener_rules \
             WHERE user_id = ?1 AND decision = 'pending' \
             ORDER BY first_seen_at DESC, sender_address ASC \
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
            .map(|(sender, first_seen_at)| ScreenerCursor {
                first_seen_at: *first_seen_at,
                sender: normalize_sender(sender),
            })
            .map(|cursor| cursor.encode())
    } else {
        None
    };

    let mut senders: Vec<ScreenerSender> = rows
        .into_iter()
        .map(|(sender, first_seen_at)| {
            ScreenerSender::fallback(normalize_sender(&sender), first_seen_at)
        })
        .collect();

    if let Err(err) = enrich_screener_senders(&state, &user, &mut senders).await {
        tracing::warn!(
            user_id = user.id,
            error = %err,
            "screener JMAP preview enrichment failed; using sidecar fallback"
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
        URL_SAFE_NO_PAD.encode(format!("{}\n{}", self.first_seen_at.to_rfc3339(), self.sender))
    }

    fn decode(value: &str) -> Result<Self, ()> {
        let decoded = URL_SAFE_NO_PAD.decode(value).map_err(|_| ())?;
        let decoded = String::from_utf8(decoded).map_err(|_| ())?;
        let Some((first_seen_at, sender)) = decoded.split_once('\n') else {
            return Err(());
        };
        let first_seen_at = DateTime::parse_from_rfc3339(first_seen_at)
            .map_err(|_| ())?
            .with_timezone(&Utc);
        let sender = normalize_sender(sender);
        if sender.is_empty() {
            return Err(());
        }

        Ok(Self {
            first_seen_at,
            sender,
        })
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
    user: &AuthUser,
    senders: &mut [ScreenerSender],
) -> Result<(), String> {
    if senders.is_empty() {
        return Ok(());
    }

    let session = jmap_session(state, user.jmap_token.clone()).await?;

    let screener_mailbox_id = mailbox_id_by_name(&session, SCREENER_MAILBOX_NAME)
        .await
        .map_err(|err| err.to_string())?
        .ok_or_else(|| format!("{SCREENER_MAILBOX_NAME} mailbox not found"))?;

    let mut sender_email_ids = Vec::with_capacity(senders.len());
    let mut email_ids = Vec::new();

    for sender in senders.iter_mut() {
        let ids =
            jmap_email_ids_from_sender_in_mailbox(&session, &sender.sender, &screener_mailbox_id)
                .await
                .map_err(|err| err.0)?;

        sender.message_count = i64::try_from(ids.len())
            .map_err(|_| "too many matching screener messages to render in the view".to_string())?;
        if ids.is_empty() {
            sender.latest_preview = None;
            sender.emails.clear();
        } else {
            email_ids.extend(ids.iter().cloned());
        }
        sender_email_ids.push(ids);
    }

    if email_ids.is_empty() {
        return Ok(());
    }

    email_ids.sort();
    email_ids.dedup();

    let mut rich_ids: Vec<String> = sender_email_ids
        .iter()
        .filter_map(|ids| ids.first().cloned())
        .collect();
    rich_ids.sort();
    rich_ids.dedup();
    let rich_id_set: HashSet<&str> = rich_ids.iter().map(String::as_str).collect();
    let light_ids: Vec<String> = email_ids
        .into_iter()
        .filter(|id| !rich_id_set.contains(id.as_str()))
        .collect();

    let mut emails_by_id = HashMap::new();
    hydrate_screener_emails(
        &session,
        &rich_ids,
        SCREENER_RICH_EMAIL_GET_CHUNK_SIZE,
        true,
        &mut emails_by_id,
    )
    .await?;
    hydrate_screener_emails(
        &session,
        &light_ids,
        SCREENER_LIGHT_EMAIL_GET_CHUNK_SIZE,
        false,
        &mut emails_by_id,
    )
    .await?;

    for (sender, ids) in senders.iter_mut().zip(sender_email_ids) {
        let emails: Vec<_> = ids
            .into_iter()
            .filter_map(|id| emails_by_id.get(&id).cloned())
            .collect();

        sender.latest_preview = emails.first().map(|(_, preview)| preview.clone());
        sender.emails = emails.into_iter().map(|(summary, _)| summary).collect();
    }

    Ok(())
}

async fn hydrate_screener_emails(
    session: &hail_jmap::Session,
    email_ids: &[String],
    chunk_size: usize,
    fetch_body_values: bool,
    emails_by_id: &mut HashMap<String, (ScreenerEmail, ScreenerLatestPreview)>,
) -> Result<(), String> {
    if email_ids.is_empty() {
        return Ok(());
    }

    for chunk in email_ids.chunks(chunk_size) {
        let mut request = session.client().build();
        let get_email = request.get_email();
        get_email
            .ids(chunk.to_vec())
            .properties(MAIL_VIEW_PROPERTIES.iter().cloned());
        if fetch_body_values {
            get_email.arguments().fetch_text_body_values(true);
            get_email.arguments().fetch_html_body_values(true);
        }
        let mut response = request
            .send_get_email()
            .await
            .map_err(|err| err.to_string())?;

        for email in response.take_list() {
            let Some(email_id) = email.id().map(ToOwned::to_owned) else {
                continue;
            };
            let received_at = email
                .received_at()
                .and_then(|ts| Utc.timestamp_opt(ts, 0).single());
            let preview_text = if fetch_body_values {
                preview_from_email(&email)
            } else {
                collapse_preview_whitespace(email.preview().unwrap_or_default())
            };
            let summary = ScreenerEmail {
                email_id: email_id.clone(),
                subject: email.subject().unwrap_or_default().to_string(),
                preview: preview_text.clone(),
                received_at,
            };
            let preview = ScreenerLatestPreview {
                subject: summary.subject.clone(),
                preview: preview_text,
                from: format_from(email.from()),
                received_at,
            };
            emails_by_id.insert(email_id, (summary, preview));
        }
    }

    Ok(())
}

fn format_from(from: Option<&[hail_jmap::jmap_client::email::EmailAddress]>) -> String {
    from.and_then(|addresses| addresses.first())
        .map(|address| match address.name() {
            Some(name) if !name.is_empty() => format!("{} <{}>", name, address.email()),
            _ => address.email().to_string(),
        })
        .unwrap_or_default()
}

async fn apply_jmap_backfill(
    state: &AppState,
    user: &AuthUser,
    sender: &str,
    decision: ScreenerDecision,
    classify_as: Option<Classification>,
) -> Result<(), ScreenerBackfillError> {
    use hail_jmap::jmap_client::mailbox::Role;

    let session = jmap_session(state, user.jmap_token.clone())
        .await
        .map_err(backfill_error)?;

    let screener_mailbox_id = mailbox_id_by_name(&session, SCREENER_MAILBOX_NAME)
        .await
        .map_err(backfill_error)?
        .ok_or_else(|| {
            ScreenerBackfillError(format!("{SCREENER_MAILBOX_NAME} mailbox not found"))
        })?;
    let target_mailbox_id = match decision {
        ScreenerDecision::Approve => jmap_mailbox_id_by_role(&session, Role::Inbox)
            .await?
            .ok_or_else(|| ScreenerBackfillError("inbox mailbox not found".to_string()))?,
        ScreenerDecision::Deny => jmap_mailbox_id_by_role(&session, Role::Trash)
            .await?
            .ok_or_else(|| ScreenerBackfillError("trash mailbox not found".to_string()))?,
    };

    let email_ids =
        jmap_email_ids_from_sender_in_mailbox(&session, sender, &screener_mailbox_id).await?;
    if email_ids.is_empty() {
        tracing::debug!(
            user_id = user.id,
            sender = %sender,
            decision = %decision.response_value(),
            "screener history backfill found no matching messages"
        );
        return Ok(());
    }

    let classification_keyword = match decision {
        ScreenerDecision::Approve => Some(
            classify_as
                .ok_or_else(|| {
                    ScreenerBackfillError(
                        "approve screener backfill missing classification".to_string(),
                    )
                })?
                .keyword(),
        ),
        ScreenerDecision::Deny => None,
    };

    let mut request = session.client().build();
    {
        let set = request.set_email();
        for email_id in &email_ids {
            let update = set.update(email_id.clone());
            update
                .mailbox_id(&screener_mailbox_id, false)
                .mailbox_id(&target_mailbox_id, true)
                .keyword("$hail_screened", true);
            if let Some(keyword) = classification_keyword {
                update.keyword(keyword, true);
            }
        }
    }

    let mut response = request.send_set_email().await.map_err(backfill_error)?;
    for email_id in &email_ids {
        response.updated(email_id).map_err(backfill_error)?;
    }

    tracing::info!(
        user_id = user.id,
        sender = %sender,
        decision = %decision.response_value(),
        moved = email_ids.len(),
        "screener history backfill applied"
    );
    Ok(())
}

async fn apply_jmap_undo_deny_backfill(
    state: &AppState,
    user: &AuthUser,
    sender: &str,
    classify_as: Classification,
) -> Result<(), ScreenerBackfillError> {
    use hail_jmap::jmap_client::mailbox::Role;

    let session = jmap_session(state, user.jmap_token.clone())
        .await
        .map_err(backfill_error)?;

    let inbox_mailbox_id = jmap_mailbox_id_by_role(&session, Role::Inbox)
        .await?
        .ok_or_else(|| ScreenerBackfillError("inbox mailbox not found".to_string()))?;
    let trash_mailbox_id = jmap_mailbox_id_by_role(&session, Role::Trash)
        .await?
        .ok_or_else(|| ScreenerBackfillError("trash mailbox not found".to_string()))?;
    let screener_mailbox_id = mailbox_id_by_name(&session, SCREENER_MAILBOX_NAME)
        .await
        .map_err(backfill_error)?;

    let mut email_ids =
        jmap_email_ids_from_sender_in_mailbox(&session, sender, &trash_mailbox_id).await?;
    if let Some(screener_mailbox_id) = &screener_mailbox_id {
        let mut screener_ids =
            jmap_email_ids_from_sender_in_mailbox(&session, sender, screener_mailbox_id).await?;
        email_ids.append(&mut screener_ids);
    }
    email_ids.sort();
    email_ids.dedup();

    if email_ids.is_empty() {
        tracing::debug!(
            user_id = user.id,
            sender = %sender,
            "screener undo deny found no matching messages"
        );
        return Ok(());
    }

    let mut request = session.client().build();
    {
        let set = request.set_email();
        for email_id in &email_ids {
            let update = set.update(email_id.clone());
            update
                .mailbox_id(&trash_mailbox_id, false)
                .mailbox_id(&inbox_mailbox_id, true)
                .keyword("$hail_screened", true)
                .keyword(classify_as.keyword(), true);
            if let Some(screener_mailbox_id) = &screener_mailbox_id {
                update.mailbox_id(screener_mailbox_id, false);
            }
            for stale_keyword in stale_classification_keywords(classify_as) {
                update.keyword(stale_keyword, false);
            }
        }
    }

    let mut response = request.send_set_email().await.map_err(backfill_error)?;
    for email_id in &email_ids {
        response.updated(email_id).map_err(backfill_error)?;
    }

    tracing::info!(
        user_id = user.id,
        sender = %sender,
        classify_as = classify_as.db_value(),
        moved = email_ids.len(),
        "screener undo deny backfill applied"
    );
    Ok(())
}

fn stale_classification_keywords(
    classify_as: Classification,
) -> impl Iterator<Item = &'static str> {
    [
        Classification::Imbox,
        Classification::Feed,
        Classification::Papertrail,
    ]
    .into_iter()
    .filter(move |candidate| *candidate != classify_as)
    .map(Classification::keyword)
}

async fn jmap_mailbox_id_by_role(
    session: &hail_jmap::Session,
    role: hail_jmap::jmap_client::mailbox::Role,
) -> Result<Option<String>, ScreenerBackfillError> {
    use hail_jmap::jmap_client::mailbox::query as mailbox_query;

    let mut query = session
        .client()
        .mailbox_query(Some(mailbox_query::Filter::role(role)), None::<Vec<_>>)
        .await
        .map_err(backfill_error)?;
    Ok(query.take_ids().into_iter().next())
}

async fn jmap_email_ids_from_sender_in_mailbox(
    session: &hail_jmap::Session,
    sender: &str,
    mailbox_id: &str,
) -> Result<Vec<String>, ScreenerBackfillError> {
    use hail_jmap::jmap_client::core::query::Filter;
    use hail_jmap::jmap_client::email::query as email_query;

    const PAGE_LIMIT: usize = 256;
    let mut ids = Vec::new();

    loop {
        let position = i32::try_from(ids.len()).map_err(|_| {
            ScreenerBackfillError("too many matching screener messages to backfill".to_string())
        })?;
        let filter = Filter::and([
            email_query::Filter::in_mailbox(mailbox_id.to_string()),
            email_query::Filter::from(sender.to_string()),
        ]);

        let mut request = session.client().build();
        request
            .query_email()
            .filter(filter)
            .sort([email_query::Comparator::received_at().descending()])
            .position(position)
            .limit(PAGE_LIMIT);
        let mut query = request.send_query_email().await.map_err(backfill_error)?;
        let mut page = query.take_ids();
        let page_len = page.len();
        ids.append(&mut page);

        if page_len < PAGE_LIMIT {
            break;
        }
    }

    Ok(ids)
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
