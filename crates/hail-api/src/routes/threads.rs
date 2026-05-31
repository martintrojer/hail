//! Thread verb endpoints.
//!
//! This module owns the `/api/threads/*` mutation surface. Mutations first
//! validate the thread id, then delegate JMAP-backed operations through small
//! injectable traits so integration tests can exercise the Axum/auth/DB path
//! without a live Stalwart server.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use chrono::{DateTime, Duration, Utc};
pub use hail_core::MailClassification as Classification;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::{OpenApiRouter, UtoipaMethodRouterExt};
use utoipa_axum::routes;

use crate::middleware::auth::AuthUser;
use crate::routes::jmap_helpers::{
    clear_thread_state, email_ids_in_thread as shared_email_ids_in_thread, jmap_session,
    move_thread_to_role as shared_move_thread_to_role, provider_error, set_thread_keyword,
    set_thread_keywords, set_thread_mailboxes, thread_action_response, validate_thread_id,
};
use crate::routes::response::{bad_request, internal, not_found};
use crate::routes::undo::{NewUndoAction, ThreadStackUndoTarget, UndoToken, create_undo_action};
use crate::state::AppState;

/// OpenAPI tag for thread mutation endpoints.
pub const TAG: &str = "threads";

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
struct StackPositionSnapshot {
    position: i64,
    added_at: DateTime<Utc>,
}

pub trait ThreadVerifier: Send + Sync + 'static {
    fn exists<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        thread_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<bool, ThreadVerifyError>> + Send + 'a>>;
}

pub trait ThreadActions: Send + Sync + 'static {
    fn current_classification<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        thread_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<Classification>, ThreadActionError>> + Send + 'a>>;

    fn classify<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        thread_id: &'a str,
        classification: Classification,
    ) -> Pin<Box<dyn Future<Output = Result<(), ThreadActionError>> + Send + 'a>>;

    fn set_keyword<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        thread_id: &'a str,
        keyword: &'static str,
        enabled: bool,
    ) -> Pin<Box<dyn Future<Output = Result<(), ThreadActionError>> + Send + 'a>>;

    fn archive<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        thread_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), ThreadActionError>> + Send + 'a>>;

    fn trash<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        thread_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), ThreadActionError>> + Send + 'a>>;

    fn spam<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        thread_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), ThreadActionError>> + Send + 'a>>;

    fn not_spam<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        thread_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), ThreadActionError>> + Send + 'a>>;

    fn restore<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        thread_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), ThreadActionError>> + Send + 'a>>;

    fn destroy<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        thread_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), ThreadActionError>> + Send + 'a>>;

    fn mark<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        thread_id: &'a str,
        read: bool,
    ) -> Pin<Box<dyn Future<Output = Result<(), ThreadActionError>> + Send + 'a>>;
}

pub struct JmapThreadVerifier;

impl ThreadVerifier for JmapThreadVerifier {
    fn exists<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        thread_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<bool, ThreadVerifyError>> + Send + 'a>> {
        Box::pin(async move {
            let session = jmap_session(state, token)
                .await
                .map_err(ThreadVerifyError)?;

            use hail_jmap::jmap_client::core::query::Filter;
            use hail_jmap::jmap_client::email::query as email_query;

            let mut response = session
                .client()
                .email_query(
                    Some(Filter::from(email_query::Filter::in_thread(thread_id))),
                    None::<
                        Vec<
                            hail_jmap::jmap_client::core::query::Comparator<
                                email_query::Comparator,
                            >,
                        >,
                    >,
                )
                .await
                .map_err(|err| ThreadVerifyError(err.to_string()))?;
            Ok(!response.take_ids().is_empty())
        })
    }
}

pub struct JmapThreadActions;

impl ThreadActions for JmapThreadActions {
    fn current_classification<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        thread_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<Classification>, ThreadActionError>> + Send + 'a>>
    {
        Box::pin(async move {
            let session = jmap_session(state, token).await.map_err(provider_error)?;
            let email_ids = email_ids_in_thread(&session, thread_id).await?;
            let mut request = session.client().build();
            request
                .get_email()
                .ids(email_ids)
                .properties([hail_jmap::jmap_client::email::Property::Keywords]);
            let mut response = request.send_get_email().await.map_err(provider_error)?;
            Ok(response.take_list().into_iter().find_map(|email| {
                Classification::ALL.into_iter().find(|candidate| {
                    email
                        .keywords()
                        .into_iter()
                        .any(|kw| kw == candidate.keyword())
                })
            }))
        })
    }

    fn classify<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        thread_id: &'a str,
        classification: Classification,
    ) -> Pin<Box<dyn Future<Output = Result<(), ThreadActionError>> + Send + 'a>> {
        Box::pin(async move {
            let session = jmap_session(state, token).await.map_err(provider_error)?;
            let inbox_id = hail_jmap::mailbox_id_by_role(
                &session,
                hail_jmap::jmap_client::mailbox::Role::Inbox,
            )
            .await
            .map_err(provider_error)?
            .ok_or_else(|| ThreadActionError::Provider("inbox mailbox not found".to_string()))?;

            set_thread_keywords(
                &session,
                thread_id,
                Classification::ALL
                    .map(|candidate| (candidate.keyword(), candidate == classification)),
            )
            .await
            .map_err(provider_error)?;
            set_thread_mailboxes(&session, thread_id, [inbox_id])
                .await
                .map_err(provider_error)?;
            Ok(())
        })
    }

    fn set_keyword<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        thread_id: &'a str,
        keyword: &'static str,
        enabled: bool,
    ) -> Pin<Box<dyn Future<Output = Result<(), ThreadActionError>> + Send + 'a>> {
        Box::pin(async move {
            let session = jmap_session(state, token).await.map_err(provider_error)?;
            set_thread_keyword(&session, thread_id, keyword, enabled)
                .await
                .map_err(provider_error)
        })
    }

    fn archive<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        thread_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), ThreadActionError>> + Send + 'a>> {
        Box::pin(async move {
            shared_move_thread_to_role(state, token, thread_id, MailboxRole::Archive)
                .await
                .map_err(provider_error)
        })
    }

    fn trash<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        thread_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), ThreadActionError>> + Send + 'a>> {
        Box::pin(async move {
            shared_move_thread_to_role(state, token, thread_id, MailboxRole::Trash)
                .await
                .map_err(provider_error)
        })
    }

    fn spam<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        thread_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), ThreadActionError>> + Send + 'a>> {
        Box::pin(async move {
            let session = jmap_session(state, token).await.map_err(provider_error)?;
            let junk_id = junk_mailbox_id(&session).await?;
            set_thread_mailboxes(&session, thread_id, [junk_id])
                .await
                .map_err(provider_error)?;
            set_thread_keyword(&session, thread_id, "$Junk", true)
                .await
                .map_err(provider_error)
        })
    }

    fn not_spam<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        thread_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), ThreadActionError>> + Send + 'a>> {
        Box::pin(async move {
            let session = jmap_session(state, token).await.map_err(provider_error)?;
            let inbox_id = hail_jmap::mailbox_id_by_role(
                &session,
                hail_jmap::jmap_client::mailbox::Role::Inbox,
            )
            .await
            .map_err(provider_error)?
            .ok_or_else(|| ThreadActionError::Provider("inbox mailbox not found".to_string()))?;

            set_thread_keyword(&session, thread_id, "$Junk", false)
                .await
                .map_err(provider_error)?;
            set_thread_keywords(
                &session,
                thread_id,
                Classification::ALL
                    .map(|candidate| (candidate.keyword(), candidate == Classification::Imbox)),
            )
            .await
            .map_err(provider_error)?;
            set_thread_mailboxes(&session, thread_id, [inbox_id])
                .await
                .map_err(provider_error)
        })
    }

    fn restore<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        thread_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), ThreadActionError>> + Send + 'a>> {
        Box::pin(async move {
            shared_move_thread_to_role(state, token, thread_id, MailboxRole::Inbox)
                .await
                .map_err(provider_error)
        })
    }

    fn destroy<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        thread_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), ThreadActionError>> + Send + 'a>> {
        Box::pin(async move {
            let session = jmap_session(state, token).await.map_err(provider_error)?;
            let email_ids = email_ids_in_thread(&session, thread_id).await?;
            let mut request = session.client().build();
            request
                .set_email()
                .destroy(email_ids.iter().map(String::as_str));
            let mut response = request.send_set_email().await.map_err(provider_error)?;
            for email_id in &email_ids {
                response.destroyed(email_id).map_err(provider_error)?;
            }
            Ok(())
        })
    }

    fn mark<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        thread_id: &'a str,
        read: bool,
    ) -> Pin<Box<dyn Future<Output = Result<(), ThreadActionError>> + Send + 'a>> {
        Box::pin(async move {
            let session = jmap_session(state, token).await.map_err(provider_error)?;
            set_thread_keyword(&session, thread_id, "$seen", read)
                .await
                .map_err(provider_error)
        })
    }
}

#[derive(Debug)]
pub struct ThreadVerifyError(String);

impl ThreadVerifyError {
    pub fn provider(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

#[derive(Debug)]
pub enum ThreadActionError {
    NotFound,
    Provider(String),
}

impl crate::routes::jmap_helpers::ProviderError for ThreadActionError {
    fn provider(message: String) -> Self {
        Self::Provider(message)
    }
}

pub fn router() -> Router<AppState> {
    Router::from(openapi_router_with_deps(
        Arc::new(JmapThreadVerifier),
        Arc::new(JmapThreadActions),
    ))
}

/// Build the OpenAPI-tracked router for production thread verbs.
pub fn openapi_router() -> OpenApiRouter<AppState> {
    openapi_router_with_deps(Arc::new(JmapThreadVerifier), Arc::new(JmapThreadActions))
}

pub fn router_with_verifier<V>(verifier: Arc<V>) -> Router<AppState>
where
    V: ThreadVerifier,
{
    Router::from(openapi_router_with_deps(
        verifier,
        Arc::new(JmapThreadActions),
    ))
}

pub fn router_with_deps<V, A>(verifier: Arc<V>, actions: Arc<A>) -> Router<AppState>
where
    V: ThreadVerifier,
    A: ThreadActions,
{
    Router::from(openapi_router_with_deps(verifier, actions))
}

fn openapi_router_with_deps<V, A>(verifier: Arc<V>, actions: Arc<A>) -> OpenApiRouter<AppState>
where
    V: ThreadVerifier,
    A: ThreadActions,
{
    let verifier: Arc<dyn ThreadVerifier> = verifier;
    let actions: Arc<dyn ThreadActions> = actions;
    OpenApiRouter::new()
        .routes(
            routes!(bubble_up)
                .layer::<_, std::convert::Infallible>(Extension(verifier))
                .layer::<_, std::convert::Infallible>(Extension(actions.clone())),
        )
        .routes(routes!(cancel_bubble_up))
        .routes(routes!(classify_thread).layer(Extension(actions.clone())))
        .routes(routes!(set_aside).layer(Extension(actions.clone())))
        .routes(routes!(reply_later).layer(Extension(actions.clone())))
        .routes(routes!(archive_thread).layer(Extension(actions.clone())))
        .routes(routes!(trash_thread).layer(Extension(actions.clone())))
        .routes(routes!(spam_thread).layer(Extension(actions.clone())))
        .routes(routes!(not_spam_thread).layer(Extension(actions.clone())))
        .routes(routes!(restore_thread).layer(Extension(actions.clone())))
        .routes(routes!(destroy_thread).layer(Extension(actions.clone())))
        .routes(routes!(mark_thread).layer(Extension(actions)))
}

#[derive(Debug, Deserialize, ToSchema)]
struct BubbleUpRequest {
    #[schema(value_type = String, format = DateTime)]
    at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
struct BubbleUpResponse {
    bubble_id: i64,
    #[schema(value_type = String, format = DateTime)]
    surface_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
struct CancelBubbleUpResponse {
    status: &'static str,
}

#[derive(Debug, Deserialize, ToSchema)]
struct ClassifyRequest {
    to: Classification,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ThreadVerbResponse {
    pub undo: Option<UndoToken>,
}

#[derive(Debug, Serialize, ToSchema)]
struct DestroyThreadResponse {
    status: &'static str,
}

#[derive(Debug, Deserialize, ToSchema)]
struct MarkRequest {
    read: bool,
}

#[utoipa::path(
    post,
    path = "/api/threads/{thread_id}/bubble-up",
    tag = TAG,
    params(
        ("thread_id" = String, Path, description = "JMAP thread id."),
    ),
    request_body(content = BubbleUpRequest, content_type = "application/json"),
    responses(
        (status = 201, description = "Thread bubble-up scheduled.", body = BubbleUpResponse),
        (status = 400, description = "Invalid bubble-up payload."),
        (status = 401, description = "Missing or invalid session."),
        (status = 404, description = "Thread not found."),
        (status = 500, description = "Bubble-up scheduling failed."),
    ),
)]
async fn bubble_up(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(verifier): Extension<Arc<dyn ThreadVerifier>>,
    Extension(actions): Extension<Arc<dyn ThreadActions>>,
    Path(thread_id): Path<String>,
    Json(body): Json<BubbleUpRequest>,
) -> Response {
    if let Err(response) = validate_thread_id(&thread_id) {
        return response;
    }

    if body.at < Utc::now() + Duration::seconds(60) {
        return bad_request("at_must_be_in_future");
    }

    let visible = match verifier
        .exists(&state, user.jmap_token.clone(), &thread_id)
        .await
    {
        Ok(visible) => visible,
        Err(err) => {
            tracing::warn!(user_id = user.id, thread_id = %thread_id, error = %err.0, "thread visibility check failed");
            return internal();
        }
    };
    if !visible {
        return not_found("not_found");
    }

    let now = Utc::now();
    let bubble_id = match sqlx::query_scalar::<_, i64>(
        "INSERT INTO bubble_ups (user_id, thread_id, surface_at, created_at) \
         VALUES (?1, ?2, ?3, ?4) RETURNING id",
    )
    .bind(user.id)
    .bind(&thread_id)
    .bind(body.at)
    .bind(now)
    .fetch_one(&state.db)
    .await
    {
        Ok(id) => id,
        Err(err) => {
            tracing::error!(user_id = user.id, thread_id = %thread_id, error = %err, "bubble-up insert failed");
            return internal();
        }
    };

    for classification in Classification::ALL {
        let _ = actions
            .set_keyword(
                &state,
                user.jmap_token.clone(),
                &thread_id,
                classification.keyword(),
                false,
            )
            .await;
    }
    for keyword in ["$hail_setaside", "$hail_replylater"] {
        let _ = actions
            .set_keyword(&state, user.jmap_token.clone(), &thread_id, keyword, false)
            .await;
    }
    if let Err(err) = hail_db::clear_thread_stack_positions(&state.db, user.id, &thread_id).await {
        tracing::error!(user_id = user.id, thread_id = %thread_id, error = %err, "thread stack cleanup failed during bubble-up schedule");
        return internal();
    }

    (
        StatusCode::CREATED,
        Json(BubbleUpResponse {
            bubble_id,
            surface_at: body.at,
        }),
    )
        .into_response()
}

#[utoipa::path(
    delete,
    path = "/api/threads/{thread_id}/bubble-up",
    tag = TAG,
    params(
        ("thread_id" = String, Path, description = "JMAP thread id."),
    ),
    responses(
        (status = 200, description = "Thread bubble-up cancelled.", body = CancelBubbleUpResponse),
        (status = 400, description = "Invalid thread id."),
        (status = 401, description = "Missing or invalid session."),
        (status = 500, description = "Bubble-up cancellation failed."),
    ),
)]
async fn cancel_bubble_up(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(thread_id): Path<String>,
) -> Response {
    if let Err(response) = validate_thread_id(&thread_id) {
        return response;
    }

    if let Err(err) = sqlx::query("DELETE FROM bubble_ups WHERE user_id = ?1 AND thread_id = ?2")
        .bind(user.id)
        .bind(&thread_id)
        .execute(&state.db)
        .await
    {
        tracing::error!(user_id = user.id, thread_id = %thread_id, error = %err, "bubble-up cancel failed");
        return internal();
    }

    Json(CancelBubbleUpResponse {
        status: "cancelled",
    })
    .into_response()
}

#[utoipa::path(
    post,
    path = "/api/threads/{thread_id}/classify",
    tag = TAG,
    params(
        ("thread_id" = String, Path, description = "JMAP thread id."),
    ),
    request_body(content = ClassifyRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "Thread reclassified.", body = ThreadVerbResponse),
        (status = 400, description = "Invalid thread id or classification."),
        (status = 401, description = "Missing or invalid session."),
        (status = 404, description = "Thread not found."),
        (status = 500, description = "Thread classify failed."),
    ),
)]
async fn classify_thread(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(actions): Extension<Arc<dyn ThreadActions>>,
    Path(thread_id): Path<String>,
    body: Result<Json<ClassifyRequest>, JsonRejection>,
) -> Response {
    if let Err(response) = validate_thread_id(&thread_id) {
        return response;
    }
    let Ok(Json(body)) = body else {
        return bad_request("invalid_classification");
    };

    thread_action_response(&user, &thread_id, || async {
        let previous_classification = actions
            .current_classification(&state, user.jmap_token.clone(), &thread_id)
            .await?;
        if previous_classification.is_none() {
            tracing::debug!(user_id = user.id, thread_id = %thread_id, "thread classify undo unavailable: previous classification keyword missing");
        }

        actions
            .classify(&state, user.jmap_token.clone(), &thread_id, body.to)
            .await?;
        for pile_keyword in ["$hail_setaside", "$hail_replylater"] {
            if let Err(err) = actions
                .set_keyword(
                    &state,
                    user.jmap_token.clone(),
                    &thread_id,
                    pile_keyword,
                    false,
                )
                .await
            {
                tracing::warn!(user_id = user.id, thread_id = %thread_id, keyword = pile_keyword, error = ?err, "failed to remove pile keyword during classify");
            }
        }
        hail_db::clear_thread_sidecar_state(&state.db, user.id, &thread_id)
            .await
            .map_err(provider_error)?;

        Ok(match previous_classification {
            Some(previous) if previous != body.to => {
                create_thread_classify_undo(&state, user.id, &thread_id, previous, body.to).await
            }
            _ => None,
        })
    })
    .await
}

#[utoipa::path(
    post,
    path = "/api/threads/{thread_id}/set-aside",
    tag = TAG,
    params(
        ("thread_id" = String, Path, description = "JMAP thread id."),
    ),
    responses(
        (status = 200, description = "Thread added to Set Aside.", body = ThreadVerbResponse),
        (status = 400, description = "Invalid thread id."),
        (status = 401, description = "Missing or invalid session."),
        (status = 404, description = "Thread not found."),
        (status = 500, description = "Set Aside failed."),
    ),
)]
async fn set_aside(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(actions): Extension<Arc<dyn ThreadActions>>,
    Path(thread_id): Path<String>,
) -> Response {
    add_to_stack(
        state,
        user,
        actions,
        thread_id,
        ThreadStackUndoTarget::SetAside,
    )
    .await
}

#[utoipa::path(
    post,
    path = "/api/threads/{thread_id}/reply-later",
    tag = TAG,
    params(
        ("thread_id" = String, Path, description = "JMAP thread id."),
    ),
    responses(
        (status = 200, description = "Thread added to Reply Later.", body = ThreadVerbResponse),
        (status = 400, description = "Invalid thread id."),
        (status = 401, description = "Missing or invalid session."),
        (status = 404, description = "Thread not found."),
        (status = 500, description = "Reply Later failed."),
    ),
)]
async fn reply_later(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(actions): Extension<Arc<dyn ThreadActions>>,
    Path(thread_id): Path<String>,
) -> Response {
    add_to_stack(
        state,
        user,
        actions,
        thread_id,
        ThreadStackUndoTarget::ReplyLater,
    )
    .await
}

async fn add_to_stack(
    state: AppState,
    user: AuthUser,
    actions: Arc<dyn ThreadActions>,
    thread_id: String,
    target: ThreadStackUndoTarget,
) -> Response {
    thread_action_response(&user, &thread_id, || async {
        let stack = target.stack();
        let keyword = target.keyword();

        actions
            .set_keyword(&state, user.jmap_token.clone(), &thread_id, keyword, true)
            .await?;

        // Remove classification keywords so the thread leaves Imbox/Feed/Paper Trail.
        for classification in Classification::ALL {
            if let Err(err) = actions
                .set_keyword(
                    &state,
                    user.jmap_token.clone(),
                    &thread_id,
                    classification.keyword(),
                    false,
                )
                .await
            {
                tracing::warn!(user_id = user.id, thread_id = %thread_id, keyword = classification.keyword(), error = ?err, "failed to remove classification keyword during stack add");
            }
        }

        let previous_position = select_stack_position(&state, user.id, stack, &thread_id)
            .await
            .map_err(provider_error)?;

        hail_db::clear_thread_sidecar_state(&state.db, user.id, &thread_id)
            .await
            .map_err(provider_error)?;

        let now = Utc::now();
        sqlx::query(
            "INSERT INTO stack_positions (user_id, stack, thread_id, position, added_at) \
             VALUES (?1, ?2, ?3, COALESCE((SELECT MAX(position) + 1 FROM stack_positions WHERE user_id = ?1 AND stack = ?2), 1), ?4) \
             ON CONFLICT(user_id, stack, thread_id) DO UPDATE SET \
               position = COALESCE((SELECT MAX(position) + 1 FROM stack_positions WHERE user_id = excluded.user_id AND stack = excluded.stack AND thread_id <> excluded.thread_id), 1), \
               added_at = excluded.added_at",
        )
        .bind(user.id)
        .bind(stack)
        .bind(&thread_id)
        .bind(now)
        .execute(&state.db)
        .await
        .map_err(provider_error)?;

        Ok(create_thread_stack_undo(&state, user.id, &thread_id, target, previous_position).await)
    })
    .await
}

#[utoipa::path(
    post,
    path = "/api/threads/{thread_id}/archive",
    tag = TAG,
    params(
        ("thread_id" = String, Path, description = "JMAP thread id."),
    ),
    responses(
        (status = 200, description = "Thread archived.", body = ThreadVerbResponse),
        (status = 400, description = "Invalid thread id."),
        (status = 401, description = "Missing or invalid session."),
        (status = 404, description = "Thread not found."),
        (status = 500, description = "Thread archive failed."),
    ),
)]
async fn archive_thread(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(actions): Extension<Arc<dyn ThreadActions>>,
    Path(thread_id): Path<String>,
) -> Response {
    thread_action_response(&user, &thread_id, || async {
        actions
            .archive(&state, user.jmap_token.clone(), &thread_id)
            .await?;
        hail_db::clear_thread_sidecar_state(&state.db, user.id, &thread_id)
            .await
            .map_err(provider_error)?;
        tracing::debug!(user_id = user.id, thread_id = %thread_id, "archive undo unavailable: previous mailbox snapshot not captured");
        Ok(None)
    })
    .await
}

#[utoipa::path(
    post,
    path = "/api/threads/{thread_id}/trash",
    tag = TAG,
    params(
        ("thread_id" = String, Path, description = "JMAP thread id."),
    ),
    responses(
        (status = 200, description = "Thread moved to trash.", body = ThreadVerbResponse),
        (status = 400, description = "Invalid thread id."),
        (status = 401, description = "Missing or invalid session."),
        (status = 404, description = "Thread not found."),
        (status = 500, description = "Thread trash failed."),
    ),
)]
async fn trash_thread(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(actions): Extension<Arc<dyn ThreadActions>>,
    Path(thread_id): Path<String>,
) -> Response {
    thread_action_response(&user, &thread_id, || async {
        actions.trash(&state, user.jmap_token.clone(), &thread_id).await?;
        clear_thread_state(
            &state,
            actions.as_ref(),
            user.jmap_token.clone(),
            user.id,
            &thread_id,
        )
        .await;
        if let Err(err) = hail_db::provider_outbound_changes::enqueue_thread_trash_change_if_bidi_enabled(
            &state.db,
            user.id,
            &thread_id,
            true,
        )
        .await
        {
            tracing::warn!(user_id = user.id, thread_id = %thread_id, error = %err, "provider outbound trash enqueue failed");
        }

        tracing::debug!(user_id = user.id, thread_id = %thread_id, "trash undo unavailable: previous mailbox snapshot not captured");
        Ok(None)
    })
    .await
}

#[utoipa::path(
    post,
    path = "/api/threads/{thread_id}/spam",
    tag = TAG,
    params(
        ("thread_id" = String, Path, description = "JMAP thread id."),
    ),
    responses(
        (status = 200, description = "Thread marked as spam.", body = ThreadVerbResponse),
        (status = 400, description = "Invalid thread id."),
        (status = 401, description = "Missing or invalid session."),
        (status = 404, description = "Thread not found."),
        (status = 500, description = "Thread spam failed."),
    ),
)]
async fn spam_thread(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(actions): Extension<Arc<dyn ThreadActions>>,
    Path(thread_id): Path<String>,
) -> Response {
    thread_action_response(&user, &thread_id, || async {
        actions.spam(&state, user.jmap_token.clone(), &thread_id).await?;
        clear_thread_state(
            &state,
            actions.as_ref(),
            user.jmap_token.clone(),
            user.id,
            &thread_id,
        )
        .await;

        tracing::debug!(user_id = user.id, thread_id = %thread_id, "spam undo unavailable: previous mailbox snapshot not captured");
        Ok(None)
    })
    .await
}

#[utoipa::path(
    post,
    path = "/api/threads/{thread_id}/not-spam",
    tag = TAG,
    params(
        ("thread_id" = String, Path, description = "JMAP thread id."),
    ),
    responses(
        (status = 200, description = "Thread marked as not spam and restored to Imbox.", body = ThreadVerbResponse),
        (status = 400, description = "Invalid thread id."),
        (status = 401, description = "Missing or invalid session."),
        (status = 404, description = "Thread not found."),
        (status = 500, description = "Thread not-spam failed."),
    ),
)]
async fn not_spam_thread(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(actions): Extension<Arc<dyn ThreadActions>>,
    Path(thread_id): Path<String>,
) -> Response {
    thread_action_response(&user, &thread_id, || async {
        actions
            .not_spam(&state, user.jmap_token.clone(), &thread_id)
            .await?;
        clear_thread_state(
            &state,
            actions.as_ref(),
            user.jmap_token.clone(),
            user.id,
            &thread_id,
        )
        .await;
        actions
            .classify(
                &state,
                user.jmap_token.clone(),
                &thread_id,
                Classification::Imbox,
            )
            .await?;

        tracing::debug!(user_id = user.id, thread_id = %thread_id, "not-spam undo unavailable: previous mailbox snapshot not captured");
        Ok(None)
    })
    .await
}

#[utoipa::path(
    post,
    path = "/api/threads/{thread_id}/restore",
    tag = TAG,
    params(
        ("thread_id" = String, Path, description = "JMAP thread id."),
    ),
    responses(
        (status = 200, description = "Thread restored to inbox.", body = ThreadVerbResponse),
        (status = 400, description = "Invalid thread id."),
        (status = 401, description = "Missing or invalid session."),
        (status = 404, description = "Thread not found."),
        (status = 500, description = "Thread restore failed."),
    ),
)]
async fn restore_thread(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(actions): Extension<Arc<dyn ThreadActions>>,
    Path(thread_id): Path<String>,
) -> Response {
    thread_action_response(&user, &thread_id, || async {
        actions
            .restore(&state, user.jmap_token.clone(), &thread_id)
            .await?;
        actions
            .classify(
                &state,
                user.jmap_token.clone(),
                &thread_id,
                Classification::Imbox,
            )
            .await?;
        hail_db::clear_thread_sidecar_state(&state.db, user.id, &thread_id)
            .await
            .map_err(provider_error)?;
        if let Err(err) = hail_db::provider_outbound_changes::enqueue_thread_trash_change_if_bidi_enabled(
            &state.db,
            user.id,
            &thread_id,
            false,
        )
        .await
        {
            tracing::warn!(user_id = user.id, thread_id = %thread_id, error = %err, "provider outbound untrash enqueue failed");
        }

        tracing::debug!(user_id = user.id, thread_id = %thread_id, "restore undo unavailable: previous mailbox snapshot not captured");
        Ok(None)
    })
    .await
}

#[utoipa::path(
    delete,
    path = "/api/threads/{thread_id}/destroy",
    tag = TAG,
    params(
        ("thread_id" = String, Path, description = "JMAP thread id."),
    ),
    responses(
        (status = 200, description = "Thread permanently destroyed.", body = DestroyThreadResponse),
        (status = 400, description = "Invalid thread id."),
        (status = 401, description = "Missing or invalid session."),
        (status = 404, description = "Thread not found."),
        (status = 500, description = "Thread destroy failed."),
    ),
)]
async fn destroy_thread(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(actions): Extension<Arc<dyn ThreadActions>>,
    Path(thread_id): Path<String>,
) -> Response {
    if let Err(response) = validate_thread_id(&thread_id) {
        return response;
    }
    match actions
        .destroy(&state, user.jmap_token.clone(), &thread_id)
        .await
    {
        Ok(()) => {
            if let Err(err) =
                sqlx::query("DELETE FROM thread_notes WHERE user_id = ?1 AND thread_id = ?2")
                    .bind(user.id)
                    .bind(&thread_id)
                    .execute(&state.db)
                    .await
            {
                tracing::error!(user_id = user.id, thread_id = %thread_id, error = %err, "thread note cleanup failed after destroy");
                return internal();
            }
            if let Err(err) = hail_db::clear_thread_sidecar_state(&state.db, user.id, &thread_id).await {
                tracing::error!(user_id = user.id, thread_id = %thread_id, error = %err, "thread sidecar cleanup failed after destroy");
                return internal();
            }
            if let Err(err) =
                sqlx::query("DELETE FROM bubble_ups WHERE user_id = ?1 AND thread_id = ?2")
                    .bind(user.id)
                    .bind(&thread_id)
                    .execute(&state.db)
                    .await
            {
                tracing::error!(user_id = user.id, thread_id = %thread_id, error = %err, "bubble-up cleanup failed after destroy");
                return internal();
            }

            Json(DestroyThreadResponse {
                status: "destroyed",
            })
            .into_response()
        }
        Err(ThreadActionError::NotFound) => not_found("not_found"),
        Err(ThreadActionError::Provider(err)) => action_internal(user.id, &thread_id, err),
    }
}

#[utoipa::path(
    post,
    path = "/api/threads/{thread_id}/mark",
    tag = TAG,
    params(
        ("thread_id" = String, Path, description = "JMAP thread id."),
    ),
    request_body(content = MarkRequest, content_type = "application/json"),
    responses(
        (status = 204, description = "Thread read/unread state updated."),
        (status = 400, description = "Invalid mark payload."),
        (status = 401, description = "Missing or invalid session."),
        (status = 404, description = "Thread not found."),
        (status = 500, description = "Thread mark failed."),
    ),
)]
async fn mark_thread(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(actions): Extension<Arc<dyn ThreadActions>>,
    Path(thread_id): Path<String>,
    body: Result<Json<MarkRequest>, JsonRejection>,
) -> Response {
    if let Err(response) = validate_thread_id(&thread_id) {
        return response;
    }
    let Ok(Json(body)) = body else {
        return bad_request("invalid_mark");
    };
    match actions
        .mark(&state, user.jmap_token.clone(), &thread_id, body.read)
        .await
    {
        Ok(()) => {
            if body.read
                && let Err(err) = hail_db::mark_thread_seen(&state.db, user.id, &thread_id).await
            {
                tracing::warn!(user_id = user.id, thread_id = %thread_id, error = %err, "failed to mark thread seen in sidecar");
            }
            if let Err(err) = hail_db::provider_outbound_changes::enqueue_thread_read_state_if_bidi_enabled(
                &state.db,
                user.id,
                &thread_id,
                body.read,
            )
            .await
            {
                tracing::warn!(user_id = user.id, thread_id = %thread_id, read = body.read, error = %err, "provider outbound read-state enqueue failed");
            }
            StatusCode::NO_CONTENT.into_response()
        }
        Err(ThreadActionError::NotFound) => not_found("not_found"),
        Err(ThreadActionError::Provider(err)) => action_internal(user.id, &thread_id, err),
    }
}

async fn create_thread_classify_undo(
    state: &AppState,
    user_id: i64,
    thread_id: &str,
    previous_classification: Classification,
    new_classification: Classification,
) -> Option<UndoToken> {
    create_undo_action(
        state,
        user_id,
        NewUndoAction::thread_classify(
            thread_id,
            previous_classification.db_value(),
            new_classification.db_value(),
        ),
    )
    .await
    .map_err(|err| {
        tracing::warn!(user_id, thread_id = %thread_id, error = %err, "undo action create failed");
        err
    })
    .ok()
}

async fn create_thread_stack_undo(
    state: &AppState,
    user_id: i64,
    thread_id: &str,
    target: ThreadStackUndoTarget,
    previous_position: Option<StackPositionSnapshot>,
) -> Option<UndoToken> {
    let previous_position = previous_position.map(|snapshot| {
        serde_json::json!({
            "position": snapshot.position,
            "added_at": snapshot.added_at,
        })
    });

    create_undo_action(
        state,
        user_id,
        NewUndoAction::thread_stack(thread_id, target, previous_position),
    )
    .await
    .map_err(|err| {
        tracing::warn!(user_id, thread_id = %thread_id, stack = target.stack(), error = %err, "undo action create failed");
        err
    })
    .ok()
}

async fn select_stack_position(
    state: &AppState,
    user_id: i64,
    stack: &str,
    thread_id: &str,
) -> Result<Option<StackPositionSnapshot>, sqlx::Error> {
    sqlx::query_as::<_, StackPositionSnapshot>(
        "SELECT position, added_at FROM stack_positions \
         WHERE user_id = ?1 AND stack = ?2 AND thread_id = ?3",
    )
    .bind(user_id)
    .bind(stack)
    .bind(thread_id)
    .fetch_optional(&state.db)
    .await
}

async fn email_ids_in_thread(
    session: &hail_jmap::Session,
    thread_id: &str,
) -> Result<Vec<String>, ThreadActionError> {
    let ids = shared_email_ids_in_thread(session, thread_id)
        .await
        .map_err(provider_error)?;
    if ids.is_empty() {
        return Err(ThreadActionError::NotFound);
    }
    Ok(ids)
}

async fn junk_mailbox_id(session: &hail_jmap::Session) -> Result<String, ThreadActionError> {
    if let Some(id) = hail_jmap::mailbox_id_by_role(
        session,
        hail_jmap::jmap_client::mailbox::Role::Junk,
    )
    .await
    .map_err(provider_error)?
    {
        return Ok(id);
    }

    let mailbox = session
        .client()
        .mailbox_create(
            "Spam",
            None::<String>,
            hail_jmap::jmap_client::mailbox::Role::Junk,
        )
        .await
        .map_err(provider_error)?;
    mailbox.id().map(str::to_string).ok_or_else(|| {
        ThreadActionError::Provider("mailbox_create returned mailbox without id".to_string())
    })
}

#[derive(Clone, Copy)]
pub enum MailboxRole {
    Archive,
    Inbox,
    Trash,
}

impl MailboxRole {
    pub const fn jmap(self) -> hail_jmap::jmap_client::mailbox::Role {
        match self {
            Self::Archive => hail_jmap::jmap_client::mailbox::Role::Archive,
            Self::Inbox => hail_jmap::jmap_client::mailbox::Role::Inbox,
            Self::Trash => hail_jmap::jmap_client::mailbox::Role::Trash,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Archive => "archive",
            Self::Inbox => "inbox",
            Self::Trash => "trash",
        }
    }
}

fn action_internal(user_id: i64, thread_id: &str, err: String) -> Response {
    tracing::warn!(user_id, thread_id, error = %err, "thread action failed");
    internal()
}
