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

use std::sync::Arc;

use async_trait::async_trait;
use axum::extract::{Extension, State, rejection::JsonRejection};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

const SCREENER_MAILBOX_NAME: &str = "Screener";
use utoipa_axum::router::{OpenApiRouter, UtoipaMethodRouterExt};
use utoipa_axum::routes;

use crate::audit;
use crate::middleware::auth::AuthUser;
use crate::routes::undo::{NewUndoAction, UndoToken, create_undo_action};
use crate::state::AppState;

/// OpenAPI tag for screener sender-review endpoints.
pub const TAG: &str = "screener";

const DEFAULT_APPROVAL_CLASSIFICATION: Classification = Classification::Imbox;

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
        .routes(routes!(post_decision).layer(Extension(backfill)))
}

#[derive(Debug, Serialize, ToSchema)]
struct ScreenerViewResponse {
    senders: Vec<ScreenerSender>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
struct ScreenerSender {
    sender: String,
    #[schema(value_type = String, format = DateTime)]
    first_seen_at: DateTime<Utc>,
    message_count: i64,
    latest_preview: Option<ScreenerLatestPreview>,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
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

    pub const fn keyword(self) -> &'static str {
        match self {
            Self::Imbox => "$hail_imbox",
            Self::Feed => "$hail_feed",
            Self::Papertrail => "$hail_papertrail",
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

#[utoipa::path(
    get,
    path = "/api/views/screener",
    tag = TAG,
    responses(
        (status = 200, description = "Pending screener senders.", body = ScreenerViewResponse),
        (status = 401, description = "Missing or invalid session."),
        (status = 500, description = "Screener lookup failed."),
    ),
)]
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

    let mut senders: Vec<ScreenerSender> = rows
        .into_iter()
        .map(|(sender, first_seen_at)| ScreenerSender::fallback(sender, first_seen_at))
        .collect();

    if let Err(err) = enrich_screener_senders(&state, &user, &mut senders).await {
        tracing::warn!(
            user_id = user.id,
            error = %err,
            "screener JMAP preview enrichment failed; using sidecar fallback"
        );
    }

    Json(ScreenerViewResponse { senders }).into_response()
}

impl ScreenerSender {
    fn fallback(sender: String, first_seen_at: DateTime<Utc>) -> Self {
        Self {
            sender,
            first_seen_at,
            message_count: 1,
            latest_preview: None,
        }
    }
}

async fn enrich_screener_senders(
    state: &AppState,
    user: &AuthUser,
    senders: &mut [ScreenerSender],
) -> Result<(), String> {
    if senders.is_empty() {
        return Ok(());
    }

    use hail_jmap::jmap_client::core::query::Filter;
    use hail_jmap::jmap_client::email::Property;
    use hail_jmap::jmap_client::email::query as email_query;
    use hail_jmap::jmap_client::mailbox::query as mailbox_query;

    let session = hail_jmap::login_bearer(&state.config.stalwart.jmap_url, user.jmap_token.clone())
        .await
        .map_err(|err| err.to_string())?;

    let mut mailbox_query = session
        .client()
        .mailbox_query(
            Some(mailbox_query::Filter::name(SCREENER_MAILBOX_NAME)),
            None::<Vec<_>>,
        )
        .await
        .map_err(|err| err.to_string())?;
    let screener_mailbox_id = mailbox_query
        .take_ids()
        .into_iter()
        .next()
        .ok_or_else(|| format!("{SCREENER_MAILBOX_NAME} mailbox not found"))?;

    for sender in senders {
        let filter = Filter::and([
            email_query::Filter::in_mailbox(screener_mailbox_id.clone()),
            email_query::Filter::from(sender.sender.clone()),
        ]);

        let mut request = session.client().build();
        request
            .query_email()
            .filter(filter)
            .sort([email_query::Comparator::received_at().descending()])
            .limit(1)
            .calculate_total(true);
        let mut query = request
            .send_query_email()
            .await
            .map_err(|err| err.to_string())?;

        sender.message_count = query
            .total()
            .and_then(|total| i64::try_from(total).ok())
            .unwrap_or(sender.message_count);

        let ids = query.take_ids();
        if ids.is_empty() {
            sender.latest_preview = None;
            continue;
        }

        let props = [
            Property::Subject,
            Property::Preview,
            Property::From,
            Property::ReceivedAt,
        ];
        let mut request = session.client().build();
        request.get_email().ids(ids).properties(props);
        let mut response = request
            .send_get_email()
            .await
            .map_err(|err| err.to_string())?;

        sender.latest_preview =
            response
                .take_list()
                .into_iter()
                .next()
                .map(|email| ScreenerLatestPreview {
                    subject: email.subject().unwrap_or_default().to_string(),
                    preview: email.preview().unwrap_or_default().to_string(),
                    from: format_from(email.from()),
                    received_at: email
                        .received_at()
                        .and_then(|ts| Utc.timestamp_opt(ts, 0).single()),
                });
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

    let session = hail_jmap::login_bearer(&state.config.stalwart.jmap_url, user.jmap_token.clone())
        .await
        .map_err(backfill_error)?;

    let screener_mailbox_id = jmap_mailbox_id_by_name(&session, SCREENER_MAILBOX_NAME)
        .await?
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

async fn jmap_mailbox_id_by_name(
    session: &hail_jmap::Session,
    name: &str,
) -> Result<Option<String>, ScreenerBackfillError> {
    use hail_jmap::jmap_client::mailbox::query as mailbox_query;

    let mut query = session
        .client()
        .mailbox_query(Some(mailbox_query::Filter::name(name)), None::<Vec<_>>)
        .await
        .map_err(backfill_error)?;
    Ok(query.take_ids().into_iter().next())
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

    if body.apply_to_history
        && let Err(err) = backfill
            .apply(&state, &user, &sender, decision, response_classify_as)
            .await
    {
        tracing::error!(user_id = user.id, sender = %sender, error = %err.0, "screener history backfill failed");
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

fn normalize_sender(sender: &str) -> String {
    sender.trim().to_ascii_lowercase()
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
