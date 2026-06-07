//! Compose and reply send pipeline.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::{OpenApiRouter, UtoipaMethodRouterExt};
use utoipa_axum::routes;

use crate::audit;
use crate::middleware::auth::{AuthSession, AuthUser};
use crate::routes::jmap_helpers::{jmap_session, provider_error};
use crate::routes::outbound::{
    OutboundHeaders, create_draft_email, looks_like_email, render_compose_body, reply_subject,
    set_rendered_body, submit_email, validate_attachments, validate_recipients, validate_subject,
};
pub use crate::routes::outbound::{OutboundMessage, ReplyHeaders};
use crate::routes::response::{bad_request, error_response, internal, not_found};
use crate::state::AppState;
use hail_db::provider_sync_audit::{
    NewProviderSyncAuditLog, ProviderSyncEventType, ProviderSyncOperationKind,
    ProviderSyncResultStatus, insert_provider_sync_audit_log,
};
use hail_worker::gmail_client::{
    CachedGmailTokenSource, GMAIL_SEND_SCOPE, GmailAccessToken, GmailAccessTokenProvider,
    GmailClientError, provider_worker_http_client,
};
use hail_worker::gmail_outbound_smtp::{
    GmailOutboundMessage, GmailOutboundSmtp, GmailOutboundSmtpClient, LettreGmailSmtpSender,
};

/// OpenAPI tag for outbound compose/reply endpoints.
pub const TAG: &str = "compose";

pub trait Composer: Send + Sync + 'static {
    fn create_draft<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        from: &'a str,
        message: OutboundMessage,
    ) -> Pin<Box<dyn Future<Output = Result<String, ComposeError>> + Send + 'a>>;

    fn submit<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        from: &'a str,
        email_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<String>, ComposeError>> + Send + 'a>>;

    fn submit_message<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        user_id: i64,
        from: &'a str,
        email_id: &'a str,
        _message: &'a OutboundMessage,
    ) -> Pin<Box<dyn Future<Output = Result<Option<String>, ComposeError>> + Send + 'a>> {
        let _ = user_id;
        self.submit(state, token, from, email_id)
    }

    fn thread_context<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        thread_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<ReplyContext>, ComposeError>> + Send + 'a>>;
}

pub struct JmapComposer;

impl Composer for JmapComposer {
    fn create_draft<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        from: &'a str,
        message: OutboundMessage,
    ) -> Pin<Box<dyn Future<Output = Result<String, ComposeError>> + Send + 'a>> {
        Box::pin(async move {
            let effective_from = message.from.as_deref().unwrap_or(from);
            let session = jmap_session(state, token).await.map_err(provider_error)?;
            create_draft_email::<ComposeError, _>(
                &session,
                OutboundHeaders::new(
                    effective_from,
                    &message.to,
                    &message.cc,
                    &message.bcc,
                    &message.subject,
                )
                .with_reply(message.reply.as_ref()),
                |email| set_rendered_body(email, &message.body),
            )
            .await
        })
    }

    fn submit<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        from: &'a str,
        email_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<String>, ComposeError>> + Send + 'a>> {
        Box::pin(async move {
            let session = jmap_session(state, token).await.map_err(provider_error)?;
            submit_email::<ComposeError>(&session, from, email_id).await
        })
    }

    fn thread_context<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        thread_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<ReplyContext>, ComposeError>> + Send + 'a>> {
        Box::pin(async move {
            let session = jmap_session(state, token).await.map_err(provider_error)?;
            use hail_jmap::jmap_client::core::query::Filter;
            use hail_jmap::jmap_client::email::query as email_query;

            let mut query = session
                .client()
                .email_query(
                    Some(Filter::from(email_query::Filter::in_thread(thread_id))),
                    Some([email_query::Comparator::received_at().ascending()]),
                )
                .await
                .map_err(provider_error)?;
            let email_ids = query.take_ids();
            if email_ids.is_empty() {
                return Ok(None);
            }
            let mut email_request = session.client().build();
            email_request
                .get_email()
                .ids(email_ids.clone())
                .properties([
                    hail_jmap::jmap_client::email::Property::Id,
                    hail_jmap::jmap_client::email::Property::Subject,
                    hail_jmap::jmap_client::email::Property::From,
                    hail_jmap::jmap_client::email::Property::MessageId,
                    hail_jmap::jmap_client::email::Property::References,
                ]);
            let mut email_response = email_request
                .send_get_email()
                .await
                .map_err(provider_error)?;
            let mut emails_by_id = email_response
                .take_list()
                .into_iter()
                .map(|email| (email.id().unwrap_or_default().to_string(), email))
                .collect::<std::collections::HashMap<_, _>>();
            let Some(last_email) = email_ids
                .into_iter()
                .rev()
                .find_map(|id| emails_by_id.remove(&id))
            else {
                return Ok(None);
            };
            let mut references = last_email
                .references()
                .map(<[String]>::to_vec)
                .unwrap_or_default();
            let in_reply_to = last_email
                .message_id()
                .map(<[String]>::to_vec)
                .unwrap_or_default();
            for id in &in_reply_to {
                if !references.iter().any(|existing| existing == id) {
                    references.push(id.clone());
                }
            }
            let subject = reply_subject(last_email.subject().unwrap_or_default());
            let to = last_email
                .from()
                .unwrap_or_default()
                .iter()
                .map(|addr| addr.email().to_string())
                .filter(|addr| looks_like_email(addr))
                .collect();
            Ok(Some(ReplyContext {
                to,
                subject,
                in_reply_to,
                references,
            }))
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplyContext {
    pub to: Vec<String>,
    pub subject: String,
    pub in_reply_to: Vec<String>,
    pub references: Vec<String>,
}
#[derive(Debug)]
pub enum ComposeError {
    SenderIdentityUnavailable,
    Provider(String),
}

impl crate::routes::jmap_helpers::ProviderError for ComposeError {
    fn provider(message: String) -> Self {
        Self::Provider(message)
    }

    fn sender_identity_unavailable() -> Self {
        Self::SenderIdentityUnavailable
    }
}

pub fn router() -> Router<AppState> {
    router_with_composer(Arc::new(ProviderRoutingComposer::new(
        JmapComposer,
        LiveGmailOutboundSmtpFactory,
    )))
}

/// Build the OpenAPI-tracked router for the production composer.
pub fn openapi_router() -> OpenApiRouter<AppState> {
    openapi_router_with_composer(Arc::new(ProviderRoutingComposer::new(
        JmapComposer,
        LiveGmailOutboundSmtpFactory,
    )))
}

pub struct ProviderRoutingComposer<C = JmapComposer, S = LiveGmailOutboundSmtpFactory> {
    inner: C,
    smtp_factory: S,
}

impl<C, S> ProviderRoutingComposer<C, S> {
    #[must_use]
    pub fn new(inner: C, smtp_factory: S) -> Self {
        Self {
            inner,
            smtp_factory,
        }
    }
}

#[derive(Default)]
pub struct LiveGmailOutboundSmtpFactory;

pub trait GmailOutboundSmtpFactory: Send + Sync + 'static {
    fn build<'a>(
        &'a self,
        state: &'a AppState,
        account: &'a ProviderOutboundAccount,
    ) -> Pin<Box<dyn Future<Output = Result<Box<dyn GmailOutboundSmtp>, ComposeError>> + Send + 'a>>;
}

impl GmailOutboundSmtpFactory for LiveGmailOutboundSmtpFactory {
    fn build<'a>(
        &'a self,
        state: &'a AppState,
        account: &'a ProviderOutboundAccount,
    ) -> Pin<Box<dyn Future<Output = Result<Box<dyn GmailOutboundSmtp>, ComposeError>> + Send + 'a>>
    {
        Box::pin(async move {
            let http = provider_worker_http_client()
                .map_err(|err| ComposeError::Provider(err.to_string()))?;
            let token_source = DbGmailOutboundTokenSource::load(
                &state.db,
                http,
                state.config.provider_import.gmail.oauth_client_id.clone(),
                state
                    .config
                    .provider_import
                    .gmail
                    .oauth_client_secret
                    .clone(),
                state
                    .config
                    .provider_import
                    .gmail
                    .oauth_token_url
                    .clone()
                    .unwrap_or_else(|| "https://oauth2.googleapis.com/token".to_string()),
                &state.server_key,
                account,
            )
            .await
            .map_err(|err| ComposeError::Provider(err.to_string()))?;
            Ok(Box::new(GmailOutboundSmtpClient::new(
                CachedGmailTokenSource::with_expiry_skew(token_source, Duration::ZERO),
                LettreGmailSmtpSender,
            )) as Box<dyn GmailOutboundSmtp>)
        })
    }
}

impl<C, S> Composer for ProviderRoutingComposer<C, S>
where
    C: Composer,
    S: GmailOutboundSmtpFactory,
{
    fn create_draft<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        from: &'a str,
        message: OutboundMessage,
    ) -> Pin<Box<dyn Future<Output = Result<String, ComposeError>> + Send + 'a>> {
        self.inner.create_draft(state, token, from, message)
    }

    fn submit<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        from: &'a str,
        email_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<String>, ComposeError>> + Send + 'a>> {
        self.inner.submit(state, token, from, email_id)
    }

    fn submit_message<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        user_id: i64,
        from: &'a str,
        email_id: &'a str,
        message: &'a OutboundMessage,
    ) -> Pin<Box<dyn Future<Output = Result<Option<String>, ComposeError>> + Send + 'a>> {
        Box::pin(async move {
            let effective_from = message.from.as_deref().unwrap_or(from);
            let Some(account) = outbound_mail_account(state, user_id, effective_from).await? else {
                return self
                    .inner
                    .submit_message(state, token, user_id, from, email_id, message)
                    .await;
            };
            match account.backend_kind {
                MailAccountBackendKind::Jmap => {
                    self.inner
                        .submit_message(state, token, user_id, from, email_id, message)
                        .await
                }
                MailAccountBackendKind::Gmail => {
                    if account.refresh_token_missing {
                        return Err(ComposeError::Provider(
                            "provider_token_missing: reconnect Gmail to enable outbound sending"
                                .to_string(),
                        ));
                    }
                    let account = account.into_provider_outbound_account()?;
                    if !account.has_gmail_send_scope() {
                        mark_provider_needs_reauth(state, &account).await?;
                        return Err(ComposeError::Provider(
                            "provider_scope_missing: reconnect Gmail to enable outbound sending"
                                .to_string(),
                        ));
                    }
                    let rfc822 = GmailOutboundMessage {
                        from: effective_from.to_string(),
                        to: message.to.clone(),
                        cc: message.cc.clone(),
                        bcc: message.bcc.clone(),
                        subject: message.subject.clone(),
                        plain_text: message.body.plain_text.clone(),
                        html: message.body.html.clone(),
                    };
                    let smtp = self.smtp_factory.build(state, &account).await?;
                    match smtp.send_gmail(&rfc822).await {
                        Ok(()) => {
                            mark_provider_sent(state, &account, email_id, &rfc822).await?;
                            Ok(Some(format!("provider:gmail:{}", account.id)))
                        }
                        Err(error) => {
                            mark_provider_send_error(
                                state,
                                &account,
                                error.error_class(),
                                &error.to_string(),
                            )
                            .await?;
                            Err(ComposeError::Provider(error.to_string()))
                        }
                    }
                }
            }
        })
    }

    fn thread_context<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        thread_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<ReplyContext>, ComposeError>> + Send + 'a>> {
        self.inner.thread_context(state, token, thread_id)
    }
}

pub fn router_with_composer<C>(composer: Arc<C>) -> Router<AppState>
where
    C: Composer,
{
    Router::from(openapi_router_with_composer(composer))
}

fn openapi_router_with_composer<C>(composer: Arc<C>) -> OpenApiRouter<AppState>
where
    C: Composer,
{
    let composer: Arc<dyn Composer> = composer;
    OpenApiRouter::new()
        .routes(routes!(compose).layer(Extension(composer.clone())))
        .routes(routes!(reply).layer(Extension(composer)))
        .routes(routes!(list_scheduled_sends))
        .routes(routes!(get_scheduled_send))
        .routes(routes!(cancel_scheduled_send))
}

#[derive(Debug, Deserialize, ToSchema)]
struct ComposePayload {
    from: Option<String>,
    to: Vec<String>,
    cc: Option<Vec<String>>,
    bcc: Option<Vec<String>>,
    subject: String,
    body_html: Option<String>,
    body_markdown: Option<String>,
    attachments: Option<Vec<serde_json::Value>>,
    #[schema(value_type = Option<String>, format = DateTime)]
    send_at: Option<DateTime<Utc>>,
}
#[derive(Debug, Deserialize, ToSchema)]
struct ReplyPayload {
    body_html: Option<String>,
    body_markdown: Option<String>,
    attachments: Option<Vec<serde_json::Value>>,
    #[schema(value_type = Option<String>, format = DateTime)]
    send_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(tag = "status")]
enum ComposeResponse {
    #[serde(rename = "pending")]
    Pending {
        scheduled_send_id: i64,
        draft_email_id: String,
    },
    #[serde(rename = "sent")]
    Sent {
        email_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        submission_id: Option<String>,
    },
}

#[derive(Debug, Serialize, ToSchema)]
struct ScheduledSendResponse {
    id: i64,
    draft_email_id: String,
    #[schema(value_type = String, format = DateTime)]
    send_at: DateTime<Utc>,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = DateTime)]
    claimed_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = DateTime)]
    sent_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[schema(value_type = String, format = DateTime)]
    created_at: DateTime<Utc>,
}

#[utoipa::path(
    post,
    path = "/api/compose",
    tag = TAG,
    request_body(content = ComposePayload, content_type = "application/json"),
    responses(
        (status = 200, description = "Message sent immediately.", body = ComposeResponse),
        (status = 201, description = "Message draft scheduled for later delivery.", body = ComposeResponse),
        (status = 400, description = "Invalid compose payload."),
        (status = 401, description = "Missing or invalid session."),
        (status = 500, description = "JMAP provider or scheduler failure."),
    ),
)]
async fn compose(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(auth_session): Extension<AuthSession>,
    Extension(composer): Extension<Arc<dyn Composer>>,
    body: Result<Json<ComposePayload>, JsonRejection>,
) -> Response {
    let Ok(Json(payload)) = body else {
        return bad_request("invalid_json");
    };
    let send_at = match validate_send_at(payload.send_at) {
        Ok(send_at) => send_at,
        Err(error) => return bad_request(error),
    };
    let message = match payload.into_message(None) {
        Ok(message) => message,
        Err(error) => return bad_request(error),
    };
    create_and_maybe_send(
        &state,
        &user,
        &auth_session,
        composer.as_ref(),
        message,
        send_at,
    )
    .await
}

#[utoipa::path(
    post,
    path = "/api/threads/{thread_id}/reply",
    tag = TAG,
    params(
        ("thread_id" = String, Path, description = "JMAP thread id to reply to."),
    ),
    request_body(content = ReplyPayload, content_type = "application/json"),
    responses(
        (status = 200, description = "Reply sent immediately.", body = ComposeResponse),
        (status = 201, description = "Reply draft scheduled for later delivery.", body = ComposeResponse),
        (status = 400, description = "Invalid thread id or reply payload."),
        (status = 401, description = "Missing or invalid session."),
        (status = 404, description = "Thread not found."),
        (status = 500, description = "JMAP provider or scheduler failure."),
    ),
)]
async fn reply(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(auth_session): Extension<AuthSession>,
    Extension(composer): Extension<Arc<dyn Composer>>,
    Path(thread_id): Path<String>,
    body: Result<Json<ReplyPayload>, JsonRejection>,
) -> Response {
    if let Err(response) = crate::routes::jmap_helpers::validate_thread_id(&thread_id) {
        return response;
    }
    let Ok(Json(payload)) = body else {
        return bad_request("invalid_json");
    };
    let send_at = match validate_send_at(payload.send_at) {
        Ok(send_at) => send_at,
        Err(error) => return bad_request(error),
    };
    let context = match composer
        .thread_context(&state, user.jmap_token.clone(), &thread_id)
        .await
    {
        Ok(Some(context)) => context,
        Ok(None) => return not_found("not_found"),
        Err(ComposeError::SenderIdentityUnavailable) => {
            return bad_request("sender_identity_unavailable");
        }
        Err(ComposeError::Provider(err)) => return provider_failed(user.id, err),
    };
    let message = match payload.into_message(context) {
        Ok(message) => message,
        Err(error) => return bad_request(error),
    };
    create_and_maybe_send(
        &state,
        &user,
        &auth_session,
        composer.as_ref(),
        message,
        send_at,
    )
    .await
}

#[utoipa::path(
    get,
    path = "/api/scheduled-sends",
    tag = TAG,
    responses(
        (status = 200, description = "Scheduled sends for the current user.", body = [ScheduledSendResponse]),
        (status = 401, description = "Missing or invalid session."),
        (status = 500, description = "Scheduled send list failed."),
    ),
)]
async fn list_scheduled_sends(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Response {
    match sqlx::query_as::<
        _,
        (
            i64,
            String,
            DateTime<Utc>,
            String,
            Option<DateTime<Utc>>,
            Option<DateTime<Utc>>,
            Option<String>,
            DateTime<Utc>,
        ),
    >(
        "SELECT id, draft_email_id, send_at, status, claimed_at, sent_at, error, created_at \
         FROM scheduled_sends \
         WHERE user_id = ? \
         ORDER BY send_at ASC, id ASC",
    )
    .bind(user.id)
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => Json(
            rows.into_iter()
                .map(scheduled_send_response_from_row)
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(err) => {
            tracing::warn!(user_id = user.id, error = %err, "scheduled send list failed");
            internal()
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/scheduled-sends/{scheduled_send_id}",
    tag = TAG,
    params(
        ("scheduled_send_id" = i64, Path, description = "Scheduled send id."),
    ),
    responses(
        (status = 200, description = "Scheduled send detail for the current user.", body = ScheduledSendResponse),
        (status = 401, description = "Missing or invalid session."),
        (status = 404, description = "Scheduled send not found."),
        (status = 500, description = "Scheduled send lookup failed."),
    ),
)]
async fn get_scheduled_send(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(scheduled_send_id): Path<i64>,
) -> Response {
    match scheduled_send_for_user(&state, user.id, scheduled_send_id).await {
        Ok(Some(row)) => Json(row).into_response(),
        Ok(None) => not_found("not_found"),
        Err(err) => {
            tracing::warn!(user_id = user.id, scheduled_send_id, error = %err, "scheduled send lookup failed");
            internal()
        }
    }
}

#[utoipa::path(
    delete,
    path = "/api/scheduled-sends/{scheduled_send_id}",
    tag = TAG,
    params(
        ("scheduled_send_id" = i64, Path, description = "Scheduled send id."),
    ),
    responses(
        (status = 200, description = "Scheduled send cancelled or already cancelled.", body = ScheduledSendResponse),
        (status = 401, description = "Missing or invalid session."),
        (status = 404, description = "Scheduled send not found."),
        (status = 409, description = "Scheduled send is not cancellable."),
        (status = 500, description = "Scheduled send cancel failed."),
    ),
)]
async fn cancel_scheduled_send(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(scheduled_send_id): Path<i64>,
) -> Response {
    let row = match scheduled_send_for_user(&state, user.id, scheduled_send_id).await {
        Ok(Some(row)) => row,
        Ok(None) => return not_found("not_found"),
        Err(err) => {
            tracing::warn!(user_id = user.id, scheduled_send_id, error = %err, "scheduled send lookup failed");
            return internal();
        }
    };

    match row.status.as_str() {
        "pending" => {}
        "cancelled" => return Json(row).into_response(),
        _ => return conflict("scheduled_send_not_cancellable"),
    }

    let now = Utc::now();
    let result = match sqlx::query(
        "UPDATE scheduled_sends \
         SET status = 'cancelled', error = NULL \
         WHERE id = ? AND user_id = ? AND status = 'pending'",
    )
    .bind(scheduled_send_id)
    .bind(user.id)
    .execute(&state.db)
    .await
    {
        Ok(result) => result,
        Err(err) => {
            tracing::warn!(user_id = user.id, scheduled_send_id, error = %err, "scheduled send cancel failed");
            return internal();
        }
    };

    if result.rows_affected() == 0 {
        return match scheduled_send_for_user(&state, user.id, scheduled_send_id).await {
            Ok(Some(row)) if row.status == "cancelled" => Json(row).into_response(),
            Ok(Some(_)) => conflict("scheduled_send_not_cancellable"),
            Ok(None) => not_found("not_found"),
            Err(err) => {
                tracing::warn!(user_id = user.id, scheduled_send_id, error = %err, "scheduled send lookup after cancel race failed");
                internal()
            }
        };
    }

    if let Err(err) = audit::record(
        &state.db,
        user.id,
        "compose.schedule_cancel",
        &serde_json::json!({
            "scheduled_send_id": scheduled_send_id,
            "cancelled_at": now,
        }),
    )
    .await
    {
        tracing::warn!(user_id = user.id, scheduled_send_id, error = %err, "audit log write failed");
    }

    match scheduled_send_for_user(&state, user.id, scheduled_send_id).await {
        Ok(Some(row)) => Json(row).into_response(),
        Ok(None) => not_found("not_found"),
        Err(err) => {
            tracing::warn!(user_id = user.id, scheduled_send_id, error = %err, "scheduled send lookup after cancel failed");
            internal()
        }
    }
}

async fn create_and_maybe_send(
    state: &AppState,
    user: &AuthUser,
    auth_session: &AuthSession,
    composer: &dyn Composer,
    message: OutboundMessage,
    send_at: Option<DateTime<Utc>>,
) -> Response {
    let draft_email_id = match composer
        .create_draft(state, user.jmap_token.clone(), &user.email, message.clone())
        .await
    {
        Ok(id) => id,
        Err(ComposeError::SenderIdentityUnavailable) => {
            return bad_request("sender_identity_unavailable");
        }
        Err(ComposeError::Provider(err)) => return provider_failed(user.id, err),
    };
    if let Some(send_at) = send_at {
        let scheduled_send_id =
            match insert_scheduled_send(state, user.id, auth_session, &draft_email_id, send_at)
                .await
            {
                Ok(id) => id,
                Err(err) => {
                    tracing::warn!(user_id = user.id, error = %err, "scheduled send insert failed");
                    return internal();
                }
            };
        if let Err(err) = audit::record(
            &state.db,
            user.id,
            "compose.schedule",
            &serde_json::json!({
                "draft_email_id": draft_email_id,
                "scheduled_send_id": scheduled_send_id,
                "send_at": send_at,
            }),
        )
        .await
        {
            tracing::warn!(user_id = user.id, error = %err, "audit log write failed");
        }
        // TODO(hail-worker): scheduled-send execution/failure is outside
        // hail-api; add matching audit rows in the worker send-later path.
        return (
            StatusCode::CREATED,
            Json(ComposeResponse::Pending {
                scheduled_send_id,
                draft_email_id,
            }),
        )
            .into_response();
    }
    match composer
        .submit_message(
            state,
            user.jmap_token.clone(),
            user.id,
            &user.email,
            &draft_email_id,
            &message,
        )
        .await
    {
        Ok(submission_id) => {
            if let Err(err) = audit::record(
                &state.db,
                user.id,
                "compose.send",
                &serde_json::json!({
                    "email_id": draft_email_id,
                    "submission_id": submission_id,
                }),
            )
            .await
            {
                tracing::warn!(user_id = user.id, error = %err, "audit log write failed");
            }
            Json(ComposeResponse::Sent {
                email_id: draft_email_id,
                submission_id,
            })
            .into_response()
        }
        Err(ComposeError::SenderIdentityUnavailable) => bad_request("sender_identity_unavailable"),
        Err(ComposeError::Provider(err)) => provider_failed(user.id, err),
    }
}

impl ComposePayload {
    fn into_message(self, reply: Option<ReplyHeaders>) -> Result<OutboundMessage, &'static str> {
        validate_attachments(&self.attachments)?;
        validate_recipients("to", &self.to, true)?;
        let cc = self.cc.unwrap_or_default();
        if !cc.is_empty() {
            validate_recipients("cc", &cc, true)?;
        }
        let bcc = self.bcc.unwrap_or_default();
        if !bcc.is_empty() {
            validate_recipients("bcc", &bcc, true)?;
        }
        validate_subject(&self.subject)?;
        let body = render_compose_body(self.body_html.as_deref(), self.body_markdown.as_deref())?;
        Ok(OutboundMessage {
            from: self.from,
            to: self.to,
            cc,
            bcc,
            subject: self.subject,
            body,
            reply,
        })
    }
}

impl ReplyPayload {
    fn into_message(self, context: ReplyContext) -> Result<OutboundMessage, &'static str> {
        validate_attachments(&self.attachments)?;
        let body = render_compose_body(self.body_html.as_deref(), self.body_markdown.as_deref())?;
        if context.to.is_empty() {
            return Err("reply_recipient_not_found");
        }
        validate_recipients("to", &context.to, true)?;
        validate_subject(&context.subject)?;
        Ok(OutboundMessage {
            from: None,
            to: context.to,
            cc: Vec::new(),
            bcc: Vec::new(),
            subject: context.subject,
            body,
            reply: Some(ReplyHeaders {
                in_reply_to: context.in_reply_to,
                references: context.references,
            }),
        })
    }
}

fn validate_send_at(send_at: Option<DateTime<Utc>>) -> Result<Option<DateTime<Utc>>, &'static str> {
    match send_at {
        Some(send_at) if send_at <= Utc::now() => Err("invalid_send_at"),
        other => Ok(other),
    }
}

async fn insert_scheduled_send(
    state: &AppState,
    user_id: i64,
    auth_session: &AuthSession,
    draft_email_id: &str,
    send_at: DateTime<Utc>,
) -> Result<i64, sqlx::Error> {
    let now = Utc::now();
    sqlx::query_scalar(
        "INSERT INTO scheduled_sends \
         (user_id, draft_email_id, send_at, status, auth_session_id, auth_session_expires_at, created_at) \
         VALUES (?1, ?2, ?3, 'pending', ?4, ?5, ?6) RETURNING id",
    )
    .bind(user_id)
    .bind(draft_email_id)
    .bind(send_at)
    .bind(auth_session.id.as_str())
    .bind(auth_session.expires_at)
    .bind(now)
    .fetch_one(&state.db)
    .await
}

async fn scheduled_send_for_user(
    state: &AppState,
    user_id: i64,
    scheduled_send_id: i64,
) -> Result<Option<ScheduledSendResponse>, sqlx::Error> {
    sqlx::query_as::<
        _,
        (
            i64,
            String,
            DateTime<Utc>,
            String,
            Option<DateTime<Utc>>,
            Option<DateTime<Utc>>,
            Option<String>,
            DateTime<Utc>,
        ),
    >(
        "SELECT id, draft_email_id, send_at, status, claimed_at, sent_at, error, created_at \
         FROM scheduled_sends \
         WHERE id = ? AND user_id = ?",
    )
    .bind(scheduled_send_id)
    .bind(user_id)
    .fetch_optional(&state.db)
    .await
    .map(|row| row.map(scheduled_send_response_from_row))
}

fn scheduled_send_response_from_row(
    row: (
        i64,
        String,
        DateTime<Utc>,
        String,
        Option<DateTime<Utc>>,
        Option<DateTime<Utc>>,
        Option<String>,
        DateTime<Utc>,
    ),
) -> ScheduledSendResponse {
    let (id, draft_email_id, send_at, status, claimed_at, sent_at, error, created_at) = row;
    ScheduledSendResponse {
        id,
        draft_email_id,
        send_at,
        status,
        claimed_at,
        sent_at,
        error,
        created_at,
    }
}

fn provider_failed(user_id: i64, err: String) -> Response {
    tracing::warn!(user_id, error = %err, "compose provider failed");
    internal()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MailAccountBackendKind {
    Gmail,
    Jmap,
}

impl MailAccountBackendKind {
    fn parse(value: &str) -> Result<Self, ComposeError> {
        match value {
            "gmail" => Ok(Self::Gmail),
            "jmap" => Ok(Self::Jmap),
            other => Err(ComposeError::Provider(format!(
                "unsupported mail account backend_kind {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone)]
struct OutboundMailAccount {
    id: i64,
    user_id: i64,
    backend_kind: MailAccountBackendKind,
    provider_account_id: String,
    provider_email: String,
    granted_scopes: Vec<String>,
    refresh_token_missing: bool,
}

impl OutboundMailAccount {
    fn into_provider_outbound_account(self) -> Result<ProviderOutboundAccount, ComposeError> {
        if self.backend_kind != MailAccountBackendKind::Gmail {
            return Err(ComposeError::Provider(
                "mail account is not a Gmail outbound account".to_string(),
            ));
        }
        Ok(ProviderOutboundAccount {
            id: self.id,
            user_id: self.user_id,
            provider_account_id: self.provider_account_id,
            provider_email: self.provider_email,
            granted_scopes: self.granted_scopes,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ProviderOutboundAccount {
    id: i64,
    user_id: i64,
    provider_account_id: String,
    provider_email: String,
    granted_scopes: Vec<String>,
}

impl ProviderOutboundAccount {
    fn has_gmail_send_scope(&self) -> bool {
        self.granted_scopes
            .iter()
            .any(|scope| scope == GMAIL_SEND_SCOPE)
    }
}

async fn outbound_mail_account(
    state: &AppState,
    user_id: i64,
    from: &str,
) -> Result<Option<OutboundMailAccount>, ComposeError> {
    let row = sqlx::query_as::<_, (i64, i64, String, String, String, String, bool)>(
        "SELECT id, user_id, backend_kind, provider_account_id, provider_email, granted_scopes_json, \
                refresh_token_enc IS NULL \
         FROM mail_accounts \
         WHERE user_id = ?1 AND lower(provider_email) = lower(?2) \
           AND ( \
             backend_kind = 'jmap' \
             OR (backend_kind = 'gmail' AND sync_status IN ('active', 'error', 'initial_sync', 'needs_reauth', 'paused')) \
           ) \
         ORDER BY CASE backend_kind WHEN 'gmail' THEN 0 WHEN 'jmap' THEN 1 ELSE 2 END, \
                  CASE WHEN sync_status = 'active' THEN 0 ELSE 1 END, id \
         LIMIT 1",
    )
    .bind(user_id)
    .bind(from.trim())
    .fetch_optional(&state.db)
    .await
    .map_err(|err| ComposeError::Provider(err.to_string()))?;
    let Some((
        id,
        user_id,
        backend_kind,
        provider_account_id,
        provider_email,
        granted_scopes_json,
        refresh_token_missing,
    )) = row
    else {
        return Ok(None);
    };
    let granted_scopes = serde_json::from_str::<Vec<String>>(&granted_scopes_json)
        .map_err(|err| ComposeError::Provider(err.to_string()))?;
    Ok(Some(OutboundMailAccount {
        id,
        user_id,
        backend_kind: MailAccountBackendKind::parse(&backend_kind)?,
        provider_account_id,
        provider_email,
        granted_scopes,
        refresh_token_missing,
    }))
}

async fn mark_provider_needs_reauth(
    state: &AppState,
    account: &ProviderOutboundAccount,
) -> Result<(), ComposeError> {
    let now = Utc::now();
    sqlx::query(
        "UPDATE mail_accounts \
         SET sync_status = 'needs_reauth', last_error_class = 'provider_scope_missing', \
             last_error_message = 'Re-authenticate Gmail to enable outbound sending', updated_at = ?1 \
         WHERE id = ?2",
    )
    .bind(now)
    .bind(account.id)
    .execute(&state.db)
    .await
    .map_err(|err| ComposeError::Provider(err.to_string()))?;
    Ok(())
}

async fn mark_provider_send_error(
    state: &AppState,
    account: &ProviderOutboundAccount,
    class: &str,
    message: &str,
) -> Result<(), ComposeError> {
    let now = Utc::now();
    sqlx::query(
        "UPDATE mail_accounts \
         SET sync_status = 'error', last_error_class = ?1, last_error_message = ?2, updated_at = ?3 \
         WHERE id = ?4",
    )
    .bind(class)
    .bind(message)
    .bind(now)
    .bind(account.id)
    .execute(&state.db)
    .await
    .map_err(|err| ComposeError::Provider(err.to_string()))?;
    Ok(())
}

async fn mark_provider_sent(
    state: &AppState,
    account: &ProviderOutboundAccount,
    email_id: &str,
    message: &GmailOutboundMessage,
) -> Result<(), ComposeError> {
    sqlx::query(
        "UPDATE mail_accounts \
         SET last_error_class = NULL, last_error_message = NULL, updated_at = ?1 \
         WHERE id = ?2",
    )
    .bind(Utc::now())
    .bind(account.id)
    .execute(&state.db)
    .await
    .map_err(|err| ComposeError::Provider(err.to_string()))?;
    let metadata = serde_json::json!({
        "email_id": email_id,
        "from": message.from,
        "provider_email": account.provider_email,
        "to_count": message.to.len(),
        "cc_count": message.cc.len(),
        "bcc_count": message.bcc.len(),
        "gmailSentPlacement": "Gmail SMTP adds sent messages to Gmail Sent automatically; hail does not IMAP-copy a duplicate."
    })
    .to_string();
    insert_provider_sync_audit_log(
        &state.db,
        NewProviderSyncAuditLog {
            user_id: account.user_id,
            provider_account_id: account.id,
            operation_kind: ProviderSyncOperationKind::OutboundSend,
            event_type: ProviderSyncEventType::SentViaProvider,
            provider_message_id: None,
            result_status: ProviderSyncResultStatus::Succeeded,
            safe_error_code: None,
            safe_error_class: None,
            safe_error_message: None,
            metadata_json: Some(&metadata),
        },
    )
    .await
    .map_err(|err| ComposeError::Provider(err.to_string()))?;
    Ok(())
}

#[derive(Debug, Clone)]
struct DbGmailOutboundTokenSource {
    http: reqwest::Client,
    client_id: Option<String>,
    client_secret: Option<SecretString>,
    token_url: String,
    refresh_token: SecretString,
}

impl DbGmailOutboundTokenSource {
    async fn load(
        db: &sqlx::SqlitePool,
        http: reqwest::Client,
        client_id: Option<String>,
        client_secret: Option<SecretString>,
        token_url: String,
        server_key: &[u8; hail_core::KEY_LEN],
        account: &ProviderOutboundAccount,
    ) -> Result<Self, sqlx::Error> {
        let ciphertext: Vec<u8> = sqlx::query_scalar(
            "SELECT refresh_token_enc FROM mail_accounts WHERE id = ?1 AND user_id = ?2",
        )
        .bind(account.id)
        .bind(account.user_id)
        .fetch_optional(db)
        .await?
        .ok_or_else(|| sqlx::Error::Protocol("mail account has no refresh token".to_string()))?;
        let context = hail_core::ProviderTokenContext::new(
            account.user_id,
            account.id,
            "gmail",
            account.provider_account_id.clone(),
            hail_core::ProviderOAuthTokenKind::Refresh,
        );
        let token = hail_core::open_provider_oauth_token(ciphertext, server_key, &context)
            .map_err(|err| sqlx::Error::Protocol(err.to_string()))?;
        Ok(Self {
            http,
            client_id,
            client_secret,
            token_url,
            refresh_token: SecretString::from(token.expose_secret().to_string()),
        })
    }
}

#[async_trait::async_trait]
impl GmailAccessTokenProvider for DbGmailOutboundTokenSource {
    async fn refresh_access_token(&self) -> Result<GmailAccessToken, GmailClientError> {
        let client_id = self.client_id.as_deref().ok_or_else(|| {
            GmailClientError::token_error(std::io::Error::other(
                "gmail oauth client id is not configured",
            ))
        })?;
        let client_secret = self.client_secret.as_ref().ok_or_else(|| {
            GmailClientError::token_error(std::io::Error::other(
                "gmail oauth client secret is not configured",
            ))
        })?;
        let body = {
            let mut form = url::form_urlencoded::Serializer::new(String::new());
            form.append_pair("client_id", client_id);
            form.append_pair("client_secret", client_secret.expose_secret());
            form.append_pair("refresh_token", self.refresh_token.expose_secret());
            form.append_pair("grant_type", "refresh_token");
            form.finish()
        };
        let token: GoogleRefreshTokenResponse = self
            .http
            .post(&self.token_url)
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .body(body)
            .send()
            .await
            .map_err(GmailClientError::Request)?
            .error_for_status()
            .map_err(GmailClientError::Request)?
            .json()
            .await
            .map_err(GmailClientError::Request)?;
        Ok(GmailAccessToken {
            token: token.access_token,
            expires_in: Duration::from_secs(token.expires_in.unwrap_or(3600)),
        })
    }
}

#[derive(Debug, serde::Deserialize)]
struct GoogleRefreshTokenResponse {
    #[serde(deserialize_with = "deserialize_secret")]
    access_token: SecretString,
    #[serde(default)]
    expires_in: Option<u64>,
}

fn deserialize_secret<'de, D>(deserializer: D) -> Result<SecretString, D::Error>
where
    D: serde::Deserializer<'de>,
{
    <String as serde::Deserialize>::deserialize(deserializer).map(SecretString::from)
}

fn conflict(error: &'static str) -> Response {
    error_response(StatusCode::CONFLICT, error)
}
