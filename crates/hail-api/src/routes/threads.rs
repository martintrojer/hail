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
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use chrono::{DateTime, Duration, Utc};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::{OpenApiRouter, UtoipaMethodRouterExt};
use utoipa_axum::routes;

use crate::middleware::auth::AuthUser;
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

    fn add_keyword<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        thread_id: &'a str,
        keyword: &'static str,
    ) -> Pin<Box<dyn Future<Output = Result<(), ThreadActionError>> + Send + 'a>>;

    fn remove_keyword<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        thread_id: &'a str,
        keyword: &'static str,
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
            let session = hail_jmap::login_bearer(&state.config.stalwart.jmap_url, token)
                .await
                .map_err(|err| ThreadVerifyError(err.to_string()))?;
            let mut request = session.client().build();
            request
                .get_thread()
                .ids([thread_id])
                .properties([hail_jmap::jmap_client::thread::Property::Id]);
            let mut response = request
                .send_get_thread()
                .await
                .map_err(|err| ThreadVerifyError(err.to_string()))?;
            Ok(!response.take_list().is_empty())
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
            let session = login(state, token).await?;
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
            let session = login(state, token).await?;
            let email_ids = email_ids_in_thread(&session, thread_id).await?;
            let inbox_id = hail_jmap::mailbox_id_by_role(
                &session,
                hail_jmap::jmap_client::mailbox::Role::Inbox,
            )
            .await
            .map_err(provider_error)?
            .ok_or_else(|| ThreadActionError::Provider("inbox mailbox not found".to_string()))?;

            for email_id in email_ids {
                for candidate in Classification::ALL {
                    session
                        .client()
                        .email_set_keyword(
                            &email_id,
                            candidate.keyword(),
                            candidate == classification,
                        )
                        .await
                        .map_err(provider_error)?;
                }
                session
                    .client()
                    .email_set_mailboxes(&email_id, [inbox_id.clone()])
                    .await
                    .map_err(provider_error)?;
            }
            Ok(())
        })
    }

    fn add_keyword<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        thread_id: &'a str,
        keyword: &'static str,
    ) -> Pin<Box<dyn Future<Output = Result<(), ThreadActionError>> + Send + 'a>> {
        Box::pin(async move {
            let session = login(state, token).await?;
            for email_id in email_ids_in_thread(&session, thread_id).await? {
                session
                    .client()
                    .email_set_keyword(&email_id, keyword, true)
                    .await
                    .map_err(provider_error)?;
            }
            Ok(())
        })
    }

    fn remove_keyword<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        thread_id: &'a str,
        keyword: &'static str,
    ) -> Pin<Box<dyn Future<Output = Result<(), ThreadActionError>> + Send + 'a>> {
        Box::pin(async move {
            let session = login(state, token).await?;
            for email_id in email_ids_in_thread(&session, thread_id).await? {
                session
                    .client()
                    .email_set_keyword(&email_id, keyword, false)
                    .await
                    .map_err(provider_error)?;
            }
            Ok(())
        })
    }

    fn archive<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        thread_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), ThreadActionError>> + Send + 'a>> {
        Box::pin(
            async move { move_thread_to_role(state, token, thread_id, MailboxRole::Archive).await },
        )
    }

    fn trash<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        thread_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), ThreadActionError>> + Send + 'a>> {
        Box::pin(
            async move { move_thread_to_role(state, token, thread_id, MailboxRole::Trash).await },
        )
    }

    fn mark<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        thread_id: &'a str,
        read: bool,
    ) -> Pin<Box<dyn Future<Output = Result<(), ThreadActionError>> + Send + 'a>> {
        Box::pin(async move {
            let session = login(state, token).await?;
            for email_id in email_ids_in_thread(&session, thread_id).await? {
                session
                    .client()
                    .email_set_keyword(&email_id, "$seen", read)
                    .await
                    .map_err(provider_error)?;
            }
            Ok(())
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

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum Classification {
    Imbox,
    Feed,
    Papertrail,
}

impl Classification {
    const ALL: [Self; 3] = [Self::Imbox, Self::Feed, Self::Papertrail];

    pub const fn keyword(self) -> &'static str {
        match self {
            Self::Imbox => "$hail_imbox",
            Self::Feed => "$hail_feed",
            Self::Papertrail => "$hail_papertrail",
        }
    }

    pub const fn db_value(self) -> &'static str {
        match self {
            Self::Imbox => "imbox",
            Self::Feed => "feed",
            Self::Papertrail => "papertrail",
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
struct ThreadVerbResponse {
    undo: Option<UndoToken>,
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
    if !looks_like_jmap_id(&thread_id) {
        return bad_request("invalid_thread_id");
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
        return not_found();
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
        if let Err(err) = actions
            .remove_keyword(
                &state,
                user.jmap_token.clone(),
                &thread_id,
                classification.keyword(),
            )
            .await
        {
            tracing::warn!(user_id = user.id, thread_id = %thread_id, keyword = classification.keyword(), error = ?err, "failed to remove classification keyword during bubble-up");
        }
    }

    for pile_keyword in ["$hail_setaside", "$hail_replylater"] {
        if let Err(err) = actions
            .remove_keyword(&state, user.jmap_token.clone(), &thread_id, pile_keyword)
            .await
        {
            tracing::warn!(user_id = user.id, thread_id = %thread_id, keyword = pile_keyword, error = ?err, "failed to remove pile keyword during bubble-up");
        }
    }

    let _ = sqlx::query("DELETE FROM stack_positions WHERE user_id = ?1 AND thread_id = ?2")
        .bind(user.id)
        .bind(&thread_id)
        .execute(&state.db)
        .await;

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
    if !looks_like_jmap_id(&thread_id) {
        return bad_request("invalid_thread_id");
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
    if !looks_like_jmap_id(&thread_id) {
        return bad_request("invalid_thread_id");
    }
    let Ok(Json(body)) = body else {
        return bad_request("invalid_classification");
    };
    let previous_classification = match actions
        .current_classification(&state, user.jmap_token.clone(), &thread_id)
        .await
    {
        Ok(Some(classification)) => Some(classification),
        Ok(None) => {
            tracing::debug!(user_id = user.id, thread_id = %thread_id, "thread classify undo unavailable: previous classification keyword missing");
            None
        }
        Err(ThreadActionError::NotFound) => return not_found(),
        Err(ThreadActionError::Provider(err)) => return action_internal(user.id, &thread_id, err),
    };
    match actions
        .classify(&state, user.jmap_token.clone(), &thread_id, body.to)
        .await
    {
        Ok(()) => {
            // Remove pile keywords + stack rows so thread leaves Set Aside / Reply Later.
            for pile_keyword in ["$hail_setaside", "$hail_replylater"] {
                if let Err(err) = actions
                    .remove_keyword(&state, user.jmap_token.clone(), &thread_id, pile_keyword)
                    .await
                {
                    tracing::warn!(user_id = user.id, thread_id = %thread_id, keyword = pile_keyword, error = ?err, "failed to remove pile keyword during classify");
                }
            }
            // Clean up sidecar stack rows.
            let _ =
                sqlx::query("DELETE FROM stack_positions WHERE user_id = ?1 AND thread_id = ?2")
                    .bind(user.id)
                    .bind(&thread_id)
                    .execute(&state.db)
                    .await;

            let undo = match previous_classification {
                Some(previous) if previous != body.to => {
                    create_thread_classify_undo(&state, user.id, &thread_id, previous, body.to)
                        .await
                }
                _ => None,
            };
            Json(ThreadVerbResponse { undo }).into_response()
        }
        Err(ThreadActionError::NotFound) => not_found(),
        Err(ThreadActionError::Provider(err)) => action_internal(user.id, &thread_id, err),
    }
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
    if !looks_like_jmap_id(&thread_id) {
        return bad_request("invalid_thread_id");
    }
    let stack = target.stack();
    let keyword = target.keyword();

    match actions
        .add_keyword(&state, user.jmap_token.clone(), &thread_id, keyword)
        .await
    {
        Ok(()) => {}
        Err(ThreadActionError::NotFound) => return not_found(),
        Err(ThreadActionError::Provider(err)) => return action_internal(user.id, &thread_id, err),
    }

    // Remove classification keywords so the thread leaves Imbox/Feed/Paper Trail.
    for classification in Classification::ALL {
        if let Err(err) = actions
            .remove_keyword(
                &state,
                user.jmap_token.clone(),
                &thread_id,
                classification.keyword(),
            )
            .await
        {
            tracing::warn!(user_id = user.id, thread_id = %thread_id, keyword = classification.keyword(), error = ?err, "failed to remove classification keyword during stack add");
        }
    }

    let previous_position = match select_stack_position(&state, user.id, stack, &thread_id).await {
        Ok(previous) => previous,
        Err(err) => {
            tracing::error!(user_id = user.id, thread_id = %thread_id, stack, error = %err, "stack position snapshot failed");
            return internal();
        }
    };

    let now = Utc::now();
    let result = sqlx::query(
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
    .await;

    match result {
        Ok(_) => {
            let undo =
                create_thread_stack_undo(&state, user.id, &thread_id, target, previous_position)
                    .await;
            Json(ThreadVerbResponse { undo }).into_response()
        }
        Err(err) => {
            tracing::error!(user_id = user.id, thread_id = %thread_id, stack, error = %err, "stack position upsert failed");
            internal()
        }
    }
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
    if !looks_like_jmap_id(&thread_id) {
        return bad_request("invalid_thread_id");
    }
    match actions
        .archive(&state, user.jmap_token.clone(), &thread_id)
        .await
    {
        Ok(()) => {
            tracing::debug!(user_id = user.id, thread_id = %thread_id, "archive undo unavailable: previous mailbox snapshot not captured");
            Json(ThreadVerbResponse { undo: None }).into_response()
        }
        Err(ThreadActionError::NotFound) => not_found(),
        Err(ThreadActionError::Provider(err)) => action_internal(user.id, &thread_id, err),
    }
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
    if !looks_like_jmap_id(&thread_id) {
        return bad_request("invalid_thread_id");
    }
    match actions
        .trash(&state, user.jmap_token.clone(), &thread_id)
        .await
    {
        Ok(()) => {
            // Remove classification keywords so thread leaves Imbox/Feed/Paper Trail.
            for classification in Classification::ALL {
                if let Err(err) = actions
                    .remove_keyword(
                        &state,
                        user.jmap_token.clone(),
                        &thread_id,
                        classification.keyword(),
                    )
                    .await
                {
                    tracing::warn!(user_id = user.id, thread_id = %thread_id, keyword = classification.keyword(), error = ?err, "failed to remove classification keyword during trash");
                }
            }
            // Remove pile keywords.
            for pile_keyword in ["$hail_setaside", "$hail_replylater"] {
                if let Err(err) = actions
                    .remove_keyword(&state, user.jmap_token.clone(), &thread_id, pile_keyword)
                    .await
                {
                    tracing::warn!(user_id = user.id, thread_id = %thread_id, keyword = pile_keyword, error = ?err, "failed to remove pile keyword during trash");
                }
            }
            // Clean up sidecar state.
            let _ =
                sqlx::query("DELETE FROM stack_positions WHERE user_id = ?1 AND thread_id = ?2")
                    .bind(user.id)
                    .bind(&thread_id)
                    .execute(&state.db)
                    .await;
            let _ = sqlx::query(
                "DELETE FROM bubble_ups WHERE user_id = ?1 AND thread_id = ?2 AND fired_at IS NULL",
            )
            .bind(user.id)
            .bind(&thread_id)
            .execute(&state.db)
            .await;

            tracing::debug!(user_id = user.id, thread_id = %thread_id, "trash undo unavailable: previous mailbox snapshot not captured");
            Json(ThreadVerbResponse { undo: None }).into_response()
        }
        Err(ThreadActionError::NotFound) => not_found(),
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
    if !looks_like_jmap_id(&thread_id) {
        return bad_request("invalid_thread_id");
    }
    let Ok(Json(body)) = body else {
        return bad_request("invalid_mark");
    };
    match actions
        .mark(&state, user.jmap_token.clone(), &thread_id, body.read)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(ThreadActionError::NotFound) => not_found(),
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

async fn login(
    state: &AppState,
    token: SecretString,
) -> Result<hail_jmap::Session, ThreadActionError> {
    hail_jmap::login_bearer(&state.config.stalwart.jmap_url, token)
        .await
        .map_err(provider_error)
}

async fn email_ids_in_thread(
    session: &hail_jmap::Session,
    thread_id: &str,
) -> Result<Vec<String>, ThreadActionError> {
    use hail_jmap::jmap_client::core::query::Filter;
    use hail_jmap::jmap_client::email::query as email_query;

    let mut query = session
        .client()
        .email_query(
            Some(Filter::from(email_query::Filter::in_thread(thread_id))),
            None::<Vec<hail_jmap::jmap_client::core::query::Comparator<email_query::Comparator>>>,
        )
        .await
        .map_err(provider_error)?;
    let ids = query.take_ids();
    if ids.is_empty() {
        return Err(ThreadActionError::NotFound);
    }
    Ok(ids)
}

#[derive(Clone, Copy)]
enum MailboxRole {
    Archive,
    Trash,
}

impl MailboxRole {
    const fn jmap(self) -> hail_jmap::jmap_client::mailbox::Role {
        match self {
            Self::Archive => hail_jmap::jmap_client::mailbox::Role::Archive,
            Self::Trash => hail_jmap::jmap_client::mailbox::Role::Trash,
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Archive => "archive",
            Self::Trash => "trash",
        }
    }
}

async fn move_thread_to_role(
    state: &AppState,
    token: SecretString,
    thread_id: &str,
    role: MailboxRole,
) -> Result<(), ThreadActionError> {
    use hail_jmap::jmap_client::mailbox::query::Filter;

    let session = login(state, token).await?;
    let mut mailbox_query = session
        .client()
        .mailbox_query(Some(Filter::role(role.jmap())), None::<Vec<_>>)
        .await
        .map_err(provider_error)?;
    let mailbox_id =
        mailbox_query.take_ids().into_iter().next().ok_or_else(|| {
            ThreadActionError::Provider(format!("{} mailbox not found", role.name()))
        })?;

    for email_id in email_ids_in_thread(&session, thread_id).await? {
        session
            .client()
            .email_set_mailboxes(&email_id, [mailbox_id.clone()])
            .await
            .map_err(provider_error)?;
    }
    Ok(())
}

fn provider_error(err: impl std::fmt::Display) -> ThreadActionError {
    ThreadActionError::Provider(err.to_string())
}

fn looks_like_jmap_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 256 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

fn bad_request(error: &'static str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        [(header::CONTENT_TYPE, "application/json")],
        format!(r#"{{"error":"{error}"}}"#),
    )
        .into_response()
}

fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"error":"not_found"}"#,
    )
        .into_response()
}

fn action_internal(user_id: i64, thread_id: &str, err: String) -> Response {
    tracing::warn!(user_id, thread_id, error = %err, "thread action failed");
    internal()
}

fn internal() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"error":"internal"}"#,
    )
        .into_response()
}
