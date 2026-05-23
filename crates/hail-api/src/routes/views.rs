//! Mail list view endpoints for Imbox, Feed, and Paper Trail.
//!
//! These views are sourced from Stalwart/JMAP by querying hail-owned
//! classification keywords (`$hail_imbox`, `$hail_feed`, `$hail_papertrail`).
//! Pending screener mail is therefore naturally excluded until the worker or
//! screener verbs classify it with one of those mutually-exclusive keywords.
//! `cursor` is accepted for API compatibility but intentionally ignored for v1;
//! responses always return `next_cursor: null`.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use axum::extract::{Extension, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use chrono::{DateTime, TimeZone, Utc};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::middleware::auth::AuthUser;
use crate::state::AppState;

const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 100;

/// Dependency-injection seam for JMAP-backed mail views.
pub trait MailViewProvider: Send + Sync + 'static {
    fn list<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        view: MailView,
        limit: usize,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<MailViewItem>, MailViewError>> + Send + 'a>>;
}

/// Production JMAP provider. It queries Email/query by the hail classification
/// keyword, sorted by `receivedAt` descending, then hydrates the returned ids
/// with the small property set needed by the list UI.
pub struct JmapMailViewProvider;

impl MailViewProvider for JmapMailViewProvider {
    fn list<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        view: MailView,
        limit: usize,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<MailViewItem>, MailViewError>> + Send + 'a>> {
        Box::pin(async move {
            use hail_jmap::jmap_client::core::query::Filter;
            use hail_jmap::jmap_client::email::Property;
            use hail_jmap::jmap_client::email::query as email_query;

            let session = hail_jmap::login_bearer(&state.config.stalwart.jmap_url, token)
                .await
                .map_err(|err| MailViewError(err.to_string()))?;

            let mut request = session.client().build();
            request
                .query_email()
                .filter(Filter::from(email_query::Filter::has_keyword(
                    view.keyword(),
                )))
                .sort([email_query::Comparator::received_at().descending()])
                .limit(limit);
            let mut query = request
                .send_query_email()
                .await
                .map_err(|err| MailViewError(err.to_string()))?;
            let ids = query.take_ids();
            if ids.is_empty() {
                return Ok(Vec::new());
            }

            let props = [
                Property::Id,
                Property::ThreadId,
                Property::From,
                Property::Subject,
                Property::Preview,
                Property::ReceivedAt,
                Property::Keywords,
            ];
            let mut request = session.client().build();
            request.get_email().ids(ids).properties(props);
            let mut response = request
                .send_get_email()
                .await
                .map_err(|err| MailViewError(err.to_string()))?;

            let classification = view.classification();
            Ok(response
                .take_list()
                .into_iter()
                .map(|email| MailViewItem {
                    thread_id: email.thread_id().unwrap_or_default().to_string(),
                    email_id: email.id().unwrap_or_default().to_string(),
                    from: format_from(email.from()),
                    subject: email.subject().unwrap_or_default().to_string(),
                    preview: email.preview().unwrap_or_default().to_string(),
                    received_at: email
                        .received_at()
                        .and_then(|ts| Utc.timestamp_opt(ts, 0).single()),
                    unread: !email.keywords().into_iter().any(|kw| kw == "$seen"),
                    classification,
                })
                .collect())
        })
    }
}

#[derive(Debug)]
pub struct MailViewError(String);

/// Build protected mail view routes.
pub fn router() -> Router<AppState> {
    router_with_provider(Arc::new(JmapMailViewProvider))
}

/// Test/helper router that injects a fake provider.
pub fn router_with_provider<P>(provider: Arc<P>) -> Router<AppState>
where
    P: MailViewProvider,
{
    Router::new()
        .route("/api/views/imbox", axum::routing::get(get_imbox::<P>))
        .route("/api/views/feed", axum::routing::get(get_feed::<P>))
        .route(
            "/api/views/papertrail",
            axum::routing::get(get_papertrail::<P>),
        )
        .layer(Extension(provider))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MailView {
    Imbox,
    Feed,
    Papertrail,
}

impl MailView {
    pub const fn keyword(self) -> &'static str {
        match self {
            Self::Imbox => "$hail_imbox",
            Self::Feed => "$hail_feed",
            Self::Papertrail => "$hail_papertrail",
        }
    }

    pub const fn classification(self) -> MailClassification {
        match self {
            Self::Imbox => MailClassification::Imbox,
            Self::Feed => MailClassification::Feed,
            Self::Papertrail => MailClassification::Papertrail,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MailClassification {
    Imbox,
    Feed,
    Papertrail,
}

#[derive(Debug, Clone, Serialize)]
pub struct MailViewItem {
    pub thread_id: String,
    pub email_id: String,
    pub from: String,
    pub subject: String,
    pub preview: String,
    pub received_at: Option<DateTime<Utc>>,
    pub unread: bool,
    pub classification: MailClassification,
}

#[derive(Debug, Deserialize)]
struct ViewQuery {
    cursor: Option<String>,
    limit: Option<usize>,
}

impl ViewQuery {
    fn normalized_limit(&self) -> usize {
        self.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT)
    }
}

#[derive(Debug, Serialize)]
struct MailViewResponse {
    items: Vec<MailViewItem>,
    next_cursor: Option<String>,
}

async fn get_imbox<P>(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(provider): Extension<Arc<P>>,
    Query(query): Query<ViewQuery>,
) -> Response
where
    P: MailViewProvider,
{
    get_view(state, user, provider, query, MailView::Imbox).await
}

async fn get_feed<P>(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(provider): Extension<Arc<P>>,
    Query(query): Query<ViewQuery>,
) -> Response
where
    P: MailViewProvider,
{
    get_view(state, user, provider, query, MailView::Feed).await
}

async fn get_papertrail<P>(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(provider): Extension<Arc<P>>,
    Query(query): Query<ViewQuery>,
) -> Response
where
    P: MailViewProvider,
{
    get_view(state, user, provider, query, MailView::Papertrail).await
}

async fn get_view<P>(
    state: AppState,
    user: AuthUser,
    provider: Arc<P>,
    query: ViewQuery,
    view: MailView,
) -> Response
where
    P: MailViewProvider,
{
    // TODO(cursor): implement opaque JMAP anchor/queryState pagination. v1 only
    // accepts and ignores the cursor parameter, returning `next_cursor: null`.
    let _cursor = query.cursor.as_deref();
    let limit = query.normalized_limit();

    let items = match provider
        .list(&state, user.jmap_token.clone(), view, limit)
        .await
    {
        Ok(items) => items,
        Err(err) => {
            tracing::warn!(user_id = user.id, view = ?view, error = %err.0, "mail view lookup failed");
            return internal();
        }
    };

    Json(MailViewResponse {
        items,
        next_cursor: None,
    })
    .into_response()
}

fn format_from(from: Option<&[hail_jmap::jmap_client::email::EmailAddress]>) -> String {
    from.and_then(|addresses| addresses.first())
        .map(|address| match address.name() {
            Some(name) if !name.is_empty() => format!("{} <{}>", name, address.email()),
            _ => address.email().to_string(),
        })
        .unwrap_or_default()
}

fn internal() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"error":"internal"}"#,
    )
        .into_response()
}
