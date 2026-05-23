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
use axum::{Json, Router, routing::post};
use chrono::{DateTime, Duration, Utc};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::middleware::auth::AuthUser;
use crate::state::AppState;

const STACK_SET_ASIDE: &str = "set_aside";
const STACK_REPLY_LATER: &str = "reply_later";

pub trait ThreadVerifier: Send + Sync + 'static {
    fn exists<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        thread_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<bool, ThreadVerifyError>> + Send + 'a>>;
}

pub trait ThreadActions: Send + Sync + 'static {
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
    fn classify<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        thread_id: &'a str,
        classification: Classification,
    ) -> Pin<Box<dyn Future<Output = Result<(), ThreadActionError>> + Send + 'a>> {
        Box::pin(async move {
            let session = login(state, token).await?;
            for email_id in email_ids_in_thread(&session, thread_id).await? {
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

#[derive(Debug)]
pub enum ThreadActionError {
    NotFound,
    Provider(String),
}

pub fn router() -> Router<AppState> {
    router_with_deps(Arc::new(JmapThreadVerifier), Arc::new(JmapThreadActions))
}

pub fn router_with_verifier<V>(verifier: Arc<V>) -> Router<AppState>
where
    V: ThreadVerifier,
{
    router_with_deps(verifier, Arc::new(JmapThreadActions))
}

pub fn router_with_deps<V, A>(verifier: Arc<V>, actions: Arc<A>) -> Router<AppState>
where
    V: ThreadVerifier,
    A: ThreadActions,
{
    Router::new()
        .route("/api/threads/{thread_id}/bubble-up", post(bubble_up::<V>))
        .route(
            "/api/threads/{thread_id}/classify",
            post(classify_thread::<A>),
        )
        .route("/api/threads/{thread_id}/set-aside", post(set_aside::<A>))
        .route(
            "/api/threads/{thread_id}/reply-later",
            post(reply_later::<A>),
        )
        .route(
            "/api/threads/{thread_id}/archive",
            post(archive_thread::<A>),
        )
        .route("/api/threads/{thread_id}/trash", post(trash_thread::<A>))
        .route("/api/threads/{thread_id}/mark", post(mark_thread::<A>))
        .layer(Extension(verifier))
        .layer(Extension(actions))
}

#[derive(Debug, Deserialize)]
struct BubbleUpRequest {
    at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct BubbleUpResponse {
    bubble_id: i64,
    surface_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
struct ClassifyRequest {
    to: Classification,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
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
}

#[derive(Debug, Deserialize)]
struct MarkRequest {
    read: bool,
}

async fn bubble_up<V>(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(verifier): Extension<Arc<V>>,
    Path(thread_id): Path<String>,
    Json(body): Json<BubbleUpRequest>,
) -> Response
where
    V: ThreadVerifier,
{
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

    (
        StatusCode::CREATED,
        Json(BubbleUpResponse {
            bubble_id,
            surface_at: body.at,
        }),
    )
        .into_response()
}

async fn classify_thread<A>(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(actions): Extension<Arc<A>>,
    Path(thread_id): Path<String>,
    body: Result<Json<ClassifyRequest>, JsonRejection>,
) -> Response
where
    A: ThreadActions,
{
    if !looks_like_jmap_id(&thread_id) {
        return bad_request("invalid_thread_id");
    }
    let Ok(Json(body)) = body else {
        return bad_request("invalid_classification");
    };
    match actions
        .classify(&state, user.jmap_token.clone(), &thread_id, body.to)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(ThreadActionError::NotFound) => not_found(),
        Err(ThreadActionError::Provider(err)) => action_internal(user.id, &thread_id, err),
    }
}

async fn set_aside<A>(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(actions): Extension<Arc<A>>,
    Path(thread_id): Path<String>,
) -> Response
where
    A: ThreadActions,
{
    add_to_stack(
        state,
        user,
        actions,
        thread_id,
        STACK_SET_ASIDE,
        "$hail_setaside",
    )
    .await
}

async fn reply_later<A>(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(actions): Extension<Arc<A>>,
    Path(thread_id): Path<String>,
) -> Response
where
    A: ThreadActions,
{
    add_to_stack(
        state,
        user,
        actions,
        thread_id,
        STACK_REPLY_LATER,
        "$hail_replylater",
    )
    .await
}

async fn add_to_stack<A>(
    state: AppState,
    user: AuthUser,
    actions: Arc<A>,
    thread_id: String,
    stack: &'static str,
    keyword: &'static str,
) -> Response
where
    A: ThreadActions,
{
    if !looks_like_jmap_id(&thread_id) {
        return bad_request("invalid_thread_id");
    }

    match actions
        .add_keyword(&state, user.jmap_token.clone(), &thread_id, keyword)
        .await
    {
        Ok(()) => {}
        Err(ThreadActionError::NotFound) => return not_found(),
        Err(ThreadActionError::Provider(err)) => return action_internal(user.id, &thread_id, err),
    }

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
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => {
            tracing::error!(user_id = user.id, thread_id = %thread_id, stack, error = %err, "stack position upsert failed");
            internal()
        }
    }
}

async fn archive_thread<A>(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(actions): Extension<Arc<A>>,
    Path(thread_id): Path<String>,
) -> Response
where
    A: ThreadActions,
{
    if !looks_like_jmap_id(&thread_id) {
        return bad_request("invalid_thread_id");
    }
    match actions
        .archive(&state, user.jmap_token.clone(), &thread_id)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(ThreadActionError::NotFound) => not_found(),
        Err(ThreadActionError::Provider(err)) => action_internal(user.id, &thread_id, err),
    }
}

async fn trash_thread<A>(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(actions): Extension<Arc<A>>,
    Path(thread_id): Path<String>,
) -> Response
where
    A: ThreadActions,
{
    if !looks_like_jmap_id(&thread_id) {
        return bad_request("invalid_thread_id");
    }
    match actions
        .trash(&state, user.jmap_token.clone(), &thread_id)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(ThreadActionError::NotFound) => not_found(),
        Err(ThreadActionError::Provider(err)) => action_internal(user.id, &thread_id, err),
    }
}

async fn mark_thread<A>(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(actions): Extension<Arc<A>>,
    Path(thread_id): Path<String>,
    Json(body): Json<MarkRequest>,
) -> Response
where
    A: ThreadActions,
{
    if !looks_like_jmap_id(&thread_id) {
        return bad_request("invalid_thread_id");
    }
    match actions
        .mark(&state, user.jmap_token.clone(), &thread_id, body.read)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(ThreadActionError::NotFound) => not_found(),
        Err(ThreadActionError::Provider(err)) => action_internal(user.id, &thread_id, err),
    }
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
