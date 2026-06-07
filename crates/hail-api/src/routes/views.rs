//! Mail list and unified search endpoints.
//!
//! Imbox, Feed, Paper Trail, Drafts, Trash, Spam, Archive, and unified
//! search are served through the cache-backed mail facade. Route tests can
//! still inject lightweight providers, but production must not bypass
//! [`hail_cache::CachedMail`].

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use axum::extract::{Extension, Query, State};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use hail_core::mail_render::FeedRenderOpts;
use hail_core::{HAIL_SPAM_KEYWORD, MailClassification};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::{OpenApiRouter, UtoipaMethodRouterExt};
use utoipa_axum::routes;

use crate::middleware::auth::AuthUser;
use crate::routes::jmap_helpers::hydrate_thread_previews;
use crate::routes::labels::LabelResponse;
use crate::routes::response::{bad_request, internal, not_found};
use crate::state::AppState;

const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 100;
const SEARCH_LIMIT: usize = 50;

/// OpenAPI tag for mail views and search endpoints.
pub const TAG: &str = "views";

pub trait MailViewProvider: Send + Sync + 'static {
    fn list<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        view: MailView,
        cursor: Option<String>,
        limit: usize,
        opts: MailViewListOpts,
    ) -> Pin<Box<dyn Future<Output = Result<MailViewPage, MailViewError>> + Send + 'a>>;

    fn count<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        view: MailView,
        unread_only: bool,
    ) -> Pin<Box<dyn Future<Output = Result<usize, MailViewError>> + Send + 'a>>;
}

pub trait SearchProvider: Send + Sync + 'static {
    fn search<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        q: &'a str,
        mailbox: Option<SearchMailbox>,
        limit: usize,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<MailSearchResult>, SearchError>> + Send + 'a>>;
}

pub struct CacheMailViewProvider;

impl MailViewProvider for CacheMailViewProvider {
    fn list<'a>(
        &'a self,
        state: &'a AppState,
        _token: SecretString,
        view: MailView,
        cursor: Option<String>,
        limit: usize,
        opts: MailViewListOpts,
    ) -> Pin<Box<dyn Future<Output = Result<MailViewPage, MailViewError>> + Send + 'a>> {
        Box::pin(async move {
            state
                .mail
                .list_view(view.into(), cursor, limit, opts.into())
                .await
                .map(Into::into)
                .map_err(|err| MailViewError::provider(err.to_string()))
        })
    }

    fn count<'a>(
        &'a self,
        state: &'a AppState,
        _token: SecretString,
        view: MailView,
        unread_only: bool,
    ) -> Pin<Box<dyn Future<Output = Result<usize, MailViewError>> + Send + 'a>> {
        Box::pin(async move {
            state
                .mail
                .count_view(view.into(), unread_only)
                .await
                .map_err(|err| MailViewError::provider(err.to_string()))
        })
    }
}

pub struct CacheSearchProvider;

impl SearchProvider for CacheSearchProvider {
    fn search<'a>(
        &'a self,
        state: &'a AppState,
        _token: SecretString,
        q: &'a str,
        mailbox: Option<SearchMailbox>,
        limit: usize,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<MailSearchResult>, SearchError>> + Send + 'a>> {
        Box::pin(async move {
            state
                .mail
                .search(q, mailbox.map(Into::into), limit)
                .await
                .map(|items| items.into_iter().map(Into::into).collect())
                .map_err(|err| SearchError::provider(err.to_string()))
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MailViewListOpts {
    pub feed_render: FeedRenderOpts,
}

#[derive(Debug, Clone)]
pub struct MailViewPage {
    pub items: Vec<MailViewItem>,
    pub next_cursor: Option<String>,
}

#[derive(Debug)]
pub struct MailViewError(String);

impl MailViewError {
    pub fn provider(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

#[derive(Debug)]
pub struct SearchError(String);

impl SearchError {
    pub fn provider(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

pub fn router() -> Router<AppState> {
    Router::from(openapi_router_with_providers(
        Arc::new(CacheMailViewProvider),
        Arc::new(CacheSearchProvider),
    ))
}

/// Build the OpenAPI-tracked router for production mail views and search.
pub fn openapi_router() -> OpenApiRouter<AppState> {
    openapi_router_with_providers(
        Arc::new(CacheMailViewProvider),
        Arc::new(CacheSearchProvider),
    )
}

pub fn router_with_provider<P>(provider: Arc<P>) -> Router<AppState>
where
    P: MailViewProvider,
{
    Router::from(openapi_router_with_providers(
        provider,
        Arc::new(EmptySearchProvider),
    ))
}

pub fn router_with_providers<P, S>(
    mail_provider: Arc<P>,
    search_provider: Arc<S>,
) -> Router<AppState>
where
    P: MailViewProvider,
    S: SearchProvider,
{
    Router::from(openapi_router_with_providers(
        mail_provider,
        search_provider,
    ))
}

fn openapi_router_with_providers<P, S>(
    mail_provider: Arc<P>,
    search_provider: Arc<S>,
) -> OpenApiRouter<AppState>
where
    P: MailViewProvider,
    S: SearchProvider,
{
    let mail_provider: Arc<dyn MailViewProvider> = mail_provider;
    let search_provider: Arc<dyn SearchProvider> = search_provider;
    OpenApiRouter::new()
        .routes(routes!(get_view_counts).layer(Extension(mail_provider.clone())))
        .routes(routes!(get_imbox).layer(Extension(mail_provider.clone())))
        .routes(routes!(get_imbox_sectioned).layer(Extension(mail_provider.clone())))
        .routes(routes!(get_feed).layer(Extension(mail_provider.clone())))
        .routes(routes!(get_papertrail).layer(Extension(mail_provider.clone())))
        .routes(routes!(get_papertrail_sectioned).layer(Extension(mail_provider.clone())))
        .routes(routes!(get_drafts).layer(Extension(mail_provider.clone())))
        .routes(routes!(get_trash).layer(Extension(mail_provider.clone())))
        .routes(routes!(get_spam).layer(Extension(mail_provider.clone())))
        .routes(routes!(get_archive).layer(Extension(mail_provider)))
        .routes(routes!(get_bubble_up))
        .routes(routes!(get_search).layer(Extension(search_provider)))
}

struct EmptySearchProvider;

impl SearchProvider for EmptySearchProvider {
    fn search<'a>(
        &'a self,
        _state: &'a AppState,
        _token: SecretString,
        _q: &'a str,
        _mailbox: Option<SearchMailbox>,
        _limit: usize,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<MailSearchResult>, SearchError>> + Send + 'a>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

impl From<MailView> for hail_cache::MailView {
    fn from(value: MailView) -> Self {
        match value {
            MailView::Imbox => Self::Imbox,
            MailView::Feed => Self::Feed,
            MailView::Papertrail => Self::Papertrail,
            MailView::Drafts => Self::Drafts,
            MailView::Trash => Self::Trash,
            MailView::Spam => Self::Spam,
            MailView::Archive => Self::Archive,
        }
    }
}

impl From<hail_cache::MailView> for MailViewClassification {
    fn from(value: hail_cache::MailView) -> Self {
        match value {
            hail_cache::MailView::Imbox => Self::Imbox,
            hail_cache::MailView::Feed => Self::Feed,
            hail_cache::MailView::Papertrail => Self::Papertrail,
            hail_cache::MailView::Drafts => Self::Drafts,
            hail_cache::MailView::Trash => Self::Trash,
            hail_cache::MailView::Spam => Self::Spam,
            hail_cache::MailView::Archive => Self::Archive,
        }
    }
}

impl From<MailViewListOpts> for hail_cache::MailViewListOpts {
    fn from(value: MailViewListOpts) -> Self {
        Self {
            feed_render: if value.feed_render.allow_remote_images {
                hail_cache::FeedRenderMode::WithRemoteImages
            } else {
                hail_cache::FeedRenderMode::WithoutRemoteImages
            },
        }
    }
}

impl From<SearchMailbox> for hail_cache::SearchMailbox {
    fn from(value: SearchMailbox) -> Self {
        match value {
            SearchMailbox::Imbox => Self::Imbox,
            SearchMailbox::Feed => Self::Feed,
            SearchMailbox::Papertrail => Self::Papertrail,
            SearchMailbox::Archive => Self::Archive,
            SearchMailbox::Trash => Self::Trash,
            SearchMailbox::Drafts => Self::Drafts,
        }
    }
}

impl From<hail_cache::MailViewPage> for MailViewPage {
    fn from(value: hail_cache::MailViewPage) -> Self {
        Self {
            items: value.items.into_iter().map(Into::into).collect(),
            next_cursor: value.next_cursor,
        }
    }
}

impl From<hail_cache::MailViewItem> for MailViewItem {
    fn from(value: hail_cache::MailViewItem) -> Self {
        Self {
            thread_id: value.thread_id,
            email_id: value.email_id,
            from: value.from,
            to: value.to,
            cc: value.cc,
            bcc: value.bcc,
            subject: value.subject,
            preview: value.preview,
            received_at: value.received_at,
            unread: value.unread,
            message_count: value.message_count,
            unread_count: value.unread_count,
            classification: value.classification.into(),
            has_notes: false,
            labels: value.labels.into_iter().map(Into::into).collect(),
            feed_html: value.feed_html,
            feed_html_with_images: value.feed_html_with_images,
            feed_blocked_trackers: value.feed_blocked_trackers.map(|trackers| {
                trackers
                    .into_iter()
                    .map(|tracker| FeedBlockedTrackerResponse {
                        src: tracker.src,
                        reason: tracker.reason,
                    })
                    .collect()
            }),
            feed_blocked_images: value.feed_blocked_images,
        }
    }
}

impl From<hail_cache::MailSearchResult> for MailSearchResult {
    fn from(value: hail_cache::MailSearchResult) -> Self {
        Self {
            thread_id: value.thread_id,
            email_id: value.email_id,
            from: value.from,
            subject: value.subject,
            preview: value.preview,
            message_count: value.message_count,
            unread_count: value.unread_count,
            unread: value.unread,
            received_at: value.received_at,
            labels: value.labels.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<hail_cache::CachedLabel> for LabelResponse {
    fn from(value: hail_cache::CachedLabel) -> Self {
        let path_segments = value.name.split('/').map(str::to_owned).collect::<Vec<_>>();
        Self {
            id: value.id,
            leaf_name: path_segments
                .last()
                .cloned()
                .unwrap_or_else(|| value.name.clone()),
            path_segments,
            name: value.name,
            source: crate::routes::labels::LabelSourceResponse::Manual,
            color: value.color,
            thread_count: 0,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
struct ViewCountsResponse {
    imbox_new: usize,
    feed_unread: usize,
    papertrail_unread: usize,
    screener_pending: usize,
    drafts: usize,
    scheduled: usize,
    set_aside: usize,
    reply_later: usize,
    bubble_up: usize,
    spam: usize,
    trash: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ToSchema)]
pub enum MailView {
    Imbox,
    Feed,
    Papertrail,
    Drafts,
    Trash,
    Spam,
    Archive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum MailViewClassification {
    Imbox,
    Feed,
    Papertrail,
    Drafts,
    Trash,
    Spam,
    Archive,
}

impl From<MailClassification> for MailViewClassification {
    fn from(value: MailClassification) -> Self {
        match value {
            MailClassification::Imbox => Self::Imbox,
            MailClassification::Feed => Self::Feed,
            MailClassification::Papertrail => Self::Papertrail,
        }
    }
}

impl MailView {
    pub const fn keyword(self) -> &'static str {
        match self {
            Self::Imbox => MailClassification::Imbox.keyword(),
            Self::Feed => MailClassification::Feed.keyword(),
            Self::Papertrail => MailClassification::Papertrail.keyword(),
            Self::Drafts => "$draft",
            Self::Trash => "$deleted",
            Self::Spam => HAIL_SPAM_KEYWORD,
            Self::Archive => "$hail_archive",
        }
    }

    pub const fn classification(self) -> MailViewClassification {
        match self {
            Self::Imbox => MailViewClassification::Imbox,
            Self::Feed => MailViewClassification::Feed,
            Self::Papertrail => MailViewClassification::Papertrail,
            Self::Drafts => MailViewClassification::Drafts,
            Self::Trash => MailViewClassification::Trash,
            Self::Spam => MailViewClassification::Spam,
            Self::Archive => MailViewClassification::Archive,
        }
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct FeedBlockedTrackerResponse {
    pub src: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MailViewItem {
    pub thread_id: String,
    pub email_id: String,
    pub from: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub preview: String,
    #[schema(value_type = Option<String>, format = DateTime)]
    pub received_at: Option<DateTime<Utc>>,
    pub unread: bool,
    pub message_count: usize,
    pub unread_count: usize,
    pub classification: MailViewClassification,
    pub has_notes: bool,
    pub labels: Vec<LabelResponse>,
    /// Sanitized, tracker-stripped HTML excerpt/body for Feed reader cards.
    /// Only populated by `/api/views/feed`; compact list views should use `preview`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feed_html: Option<String>,
    /// Sanitized Feed body with remote images enabled for a per-card or global
    /// user opt-in. Only populated by `/api/views/feed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feed_html_with_images: Option<String>,
    /// Tracker removals observed while rendering Feed HTML.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feed_blocked_trackers: Option<Vec<FeedBlockedTrackerResponse>>,
    /// Regular remote images removed while the Feed image preference is off.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feed_blocked_images: Option<usize>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MailSearchResult {
    pub thread_id: String,
    pub email_id: String,
    pub from: String,
    pub subject: String,
    pub preview: String,
    pub message_count: usize,
    pub unread_count: usize,
    pub unread: bool,
    #[schema(value_type = Option<String>, format = DateTime)]
    pub received_at: Option<DateTime<Utc>>,
    pub labels: Vec<LabelResponse>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BubbleUpViewItem {
    pub bubble_id: i64,
    pub thread_id: String,
    #[schema(value_type = String, format = DateTime)]
    pub surface_at: DateTime<Utc>,
    #[schema(value_type = String, format = DateTime)]
    pub created_at: DateTime<Utc>,
    pub from: String,
    pub subject: String,
    pub preview: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
enum SearchResult {
    Mail {
        thread_id: String,
        email_id: String,
        from: String,
        subject: String,
        preview: String,
        message_count: usize,
        unread_count: usize,
        unread: bool,
        #[schema(value_type = Option<String>, format = DateTime)]
        received_at: Option<DateTime<Utc>>,
        labels: Vec<LabelResponse>,
    },
    ContactNote {
        address: String,
        markdown: String,
        #[schema(value_type = String, format = DateTime)]
        updated_at: DateTime<Utc>,
    },
}

impl From<MailSearchResult> for SearchResult {
    fn from(item: MailSearchResult) -> Self {
        Self::Mail {
            thread_id: item.thread_id,
            email_id: item.email_id,
            from: item.from,
            subject: item.subject,
            preview: item.preview,
            message_count: item.message_count,
            unread_count: item.unread_count,
            unread: item.unread,
            received_at: item.received_at,
            labels: item.labels,
        }
    }
}

#[derive(Debug, Deserialize, IntoParams)]
struct ViewQuery {
    cursor: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
struct SearchQuery {
    q: Option<String>,
    scope: Option<String>,
    #[param(example = "imbox")]
    mailbox: Option<String>,
    #[param(example = 12)]
    label_id: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchMailbox {
    Imbox,
    Feed,
    Papertrail,
    Archive,
    Trash,
    Drafts,
}

impl SearchMailbox {
    fn parse(value: Option<&str>) -> Result<Option<Self>, ()> {
        match value.unwrap_or("all") {
            "all" => Ok(None),
            "imbox" => Ok(Some(Self::Imbox)),
            "feed" => Ok(Some(Self::Feed)),
            "papertrail" => Ok(Some(Self::Papertrail)),
            "archive" => Ok(Some(Self::Archive)),
            "trash" => Ok(Some(Self::Trash)),
            "drafts" => Ok(Some(Self::Drafts)),
            _ => Err(()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SearchScope {
    Mail,
    Notes,
    All,
}

impl SearchScope {
    fn parse(value: Option<&str>) -> Option<Self> {
        match value.unwrap_or("all") {
            "mail" => Some(Self::Mail),
            "notes" => Some(Self::Notes),
            "all" => Some(Self::All),
            _ => None,
        }
    }

    const fn includes_mail(self) -> bool {
        matches!(self, Self::Mail | Self::All)
    }

    const fn includes_notes(self) -> bool {
        matches!(self, Self::Notes | Self::All)
    }
}

impl ViewQuery {
    fn normalized_limit(&self) -> usize {
        self.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT)
    }
}

#[derive(Debug, Serialize, ToSchema)]
struct MailViewResponse {
    items: Vec<MailViewItem>,
    next_cursor: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
struct ImboxSectionedResponse {
    bubbled_up: Vec<MailViewItem>,
    new_for_you: Vec<MailViewItem>,
    previously_seen: Vec<MailViewItem>,
    new_count: usize,
    previously_seen_total: usize,
    next_cursor: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
struct SectionedMailViewResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    bubble_up: Option<Vec<MailViewItem>>,
    new: Vec<MailViewItem>,
    seen: Vec<MailViewItem>,
    next_cursor: Option<String>,
}

struct LoadedSectionedMailView {
    response: SectionedMailViewResponse,
    seen_total: usize,
}

#[derive(Debug, Serialize, ToSchema)]
struct BubbleUpViewResponse {
    items: Vec<BubbleUpViewItem>,
}

#[derive(Debug, Serialize, ToSchema)]
struct SearchResponse {
    results: Vec<SearchResult>,
}

#[utoipa::path(
    get,
    path = "/api/views/counts",
    tag = TAG,
    responses(
        (status = 200, description = "Cheap sidebar navigation counts for the current user.", body = ViewCountsResponse),
        (status = 401, description = "Missing or invalid session."),
        (status = 500, description = "View count lookup failed."),
    ),
)]
async fn get_view_counts(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(provider): Extension<Arc<dyn MailViewProvider>>,
) -> Response {
    match load_view_counts(&state, &user, provider).await {
        Ok(counts) => Json(counts).into_response(),
        Err(err) => {
            tracing::warn!(user_id = user.id, error = %err, "view count lookup failed");
            internal()
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/views/imbox",
    tag = TAG,
    params(ViewQuery),
    responses(
        (status = 200, description = "Imbox mail view.", body = MailViewResponse),
        (status = 401, description = "Missing or invalid session."),
        (status = 500, description = "JMAP mail view lookup failed."),
    ),
)]
async fn get_imbox(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(provider): Extension<Arc<dyn MailViewProvider>>,
    Query(query): Query<ViewQuery>,
) -> Response {
    get_view(state, user, provider, query, MailView::Imbox).await
}

#[utoipa::path(
    get,
    path = "/api/views/imbox/sectioned",
    tag = TAG,
    params(ViewQuery),
    responses(
        (status = 200, description = "Imbox mail view partitioned into Bubble Up, new, and seen sections.", body = ImboxSectionedResponse),
        (status = 401, description = "Missing or invalid session."),
        (status = 500, description = "Imbox sectioned view lookup failed."),
    ),
)]
async fn get_imbox_sectioned(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(provider): Extension<Arc<dyn MailViewProvider>>,
    Query(query): Query<ViewQuery>,
) -> Response {
    get_imbox_sectioned_view(state, user, provider, query).await
}

#[utoipa::path(
    get,
    path = "/api/views/feed",
    tag = TAG,
    params(ViewQuery),
    responses(
        (status = 200, description = "Feed mail view.", body = MailViewResponse),
        (status = 401, description = "Missing or invalid session."),
        (status = 500, description = "JMAP mail view lookup failed."),
    ),
)]
async fn get_feed(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(provider): Extension<Arc<dyn MailViewProvider>>,
    Query(query): Query<ViewQuery>,
) -> Response {
    get_view(state, user, provider, query, MailView::Feed).await
}

#[utoipa::path(
    get,
    path = "/api/views/papertrail",
    tag = TAG,
    params(ViewQuery),
    responses(
        (status = 200, description = "Paper Trail mail view.", body = MailViewResponse),
        (status = 401, description = "Missing or invalid session."),
        (status = 500, description = "JMAP mail view lookup failed."),
    ),
)]
async fn get_papertrail(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(provider): Extension<Arc<dyn MailViewProvider>>,
    Query(query): Query<ViewQuery>,
) -> Response {
    get_view(state, user, provider, query, MailView::Papertrail).await
}

#[utoipa::path(
    get,
    path = "/api/views/papertrail/sectioned",
    tag = TAG,
    params(ViewQuery),
    responses(
        (status = 200, description = "Paper Trail mail view partitioned into unread and read sections.", body = SectionedMailViewResponse),
        (status = 401, description = "Missing or invalid session."),
        (status = 500, description = "Paper Trail sectioned view lookup failed."),
    ),
)]
async fn get_papertrail_sectioned(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(provider): Extension<Arc<dyn MailViewProvider>>,
    Query(query): Query<ViewQuery>,
) -> Response {
    match get_unread_sectioned_view(
        &state,
        &user,
        provider,
        query,
        MailView::Papertrail,
        false,
        false,
        true,
        MAX_LIMIT,
        None,
    )
    .await
    {
        Ok(sectioned) => Json(sectioned.response).into_response(),
        Err(SectionedViewError::Provider(err)) => {
            tracing::warn!(user_id = user.id, error = %err.0, "papertrail sectioned view lookup failed");
            internal()
        }
        Err(SectionedViewError::Sqlx(err)) => {
            tracing::error!(user_id = user.id, error = %err, "papertrail sectioned metadata lookup failed");
            internal()
        }
        Err(SectionedViewError::Labels(err)) => {
            tracing::error!(user_id = user.id, error = %err, "papertrail sectioned label lookup failed");
            internal()
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/views/drafts",
    tag = TAG,
    params(ViewQuery),
    responses(
        (status = 200, description = "Drafts mail view.", body = MailViewResponse),
        (status = 401, description = "Missing or invalid session."),
        (status = 500, description = "JMAP mail view lookup failed."),
    ),
)]
async fn get_drafts(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(provider): Extension<Arc<dyn MailViewProvider>>,
    Query(query): Query<ViewQuery>,
) -> Response {
    get_view(state, user, provider, query, MailView::Drafts).await
}

#[utoipa::path(
    get,
    path = "/api/views/trash",
    tag = TAG,
    params(ViewQuery),
    responses(
        (status = 200, description = "Trash mail view.", body = MailViewResponse),
        (status = 401, description = "Missing or invalid session."),
        (status = 500, description = "JMAP mail view lookup failed."),
    ),
)]
async fn get_trash(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(provider): Extension<Arc<dyn MailViewProvider>>,
    Query(query): Query<ViewQuery>,
) -> Response {
    get_view(state, user, provider, query, MailView::Trash).await
}

#[utoipa::path(
    get,
    path = "/api/views/spam",
    tag = TAG,
    params(ViewQuery),
    responses(
        (status = 200, description = "Spam mail view.", body = MailViewResponse),
        (status = 401, description = "Missing or invalid session."),
        (status = 500, description = "JMAP mail view lookup failed."),
    ),
)]
async fn get_spam(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(provider): Extension<Arc<dyn MailViewProvider>>,
    Query(query): Query<ViewQuery>,
) -> Response {
    get_view(state, user, provider, query, MailView::Spam).await
}

#[utoipa::path(
    get,
    path = "/api/views/archive",
    tag = TAG,
    params(ViewQuery),
    responses(
        (status = 200, description = "Archive mail view.", body = MailViewResponse),
        (status = 401, description = "Missing or invalid session."),
        (status = 500, description = "JMAP mail view lookup failed."),
    ),
)]
async fn get_archive(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(provider): Extension<Arc<dyn MailViewProvider>>,
    Query(query): Query<ViewQuery>,
) -> Response {
    get_view(state, user, provider, query, MailView::Archive).await
}

#[utoipa::path(
    get,
    path = "/api/views/bubble-up",
    tag = TAG,
    responses(
        (status = 200, description = "Scheduled future Bubble Up entries.", body = BubbleUpViewResponse),
        (status = 401, description = "Missing or invalid session."),
        (status = 500, description = "Bubble Up view lookup failed."),
    ),
)]
async fn get_bubble_up(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Response {
    match list_bubble_ups(&state, user.id).await {
        Ok(mut items) => {
            let previews = hydrate_thread_previews(
                &state,
                user.id,
                user.jmap_token.clone(),
                "bubble_up",
                items.iter().map(|item| item.thread_id.clone()),
            )
            .await;
            for item in &mut items {
                if let Some(preview) = previews.get(&item.thread_id) {
                    item.from = preview.from.clone();
                    item.subject = preview.subject.clone();
                    item.preview = preview.preview.clone();
                }
            }
            Json(BubbleUpViewResponse { items }).into_response()
        }
        Err(err) => {
            tracing::error!(user_id = user.id, error = %err, "bubble-up view lookup failed");
            internal()
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/views/search",
    tag = TAG,
    params(SearchQuery),
    responses(
        (status = 200, description = "Unified mail/contact-note search.", body = SearchResponse),
        (status = 400, description = "Invalid search query."),
        (status = 401, description = "Missing or invalid session."),
        (status = 500, description = "Search lookup failed."),
    ),
)]
async fn get_search(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(provider): Extension<Arc<dyn SearchProvider>>,
    Query(query): Query<SearchQuery>,
) -> Response {
    let q = match query.q.as_deref().map(str::trim) {
        Some(q) if q.chars().count() >= 2 => q,
        _ => return bad_request("q_min_length"),
    };
    if query.scope.as_deref() == Some("clips") {
        return bad_request("clips_unsupported");
    }
    let scope = match SearchScope::parse(query.scope.as_deref()) {
        Some(scope) => scope,
        None => return bad_request("invalid_scope"),
    };

    let mailbox = match SearchMailbox::parse(query.mailbox.as_deref()) {
        Ok(mailbox) => mailbox,
        Err(()) => return bad_request("invalid_mailbox"),
    };

    let label_id = match query.label_id {
        Some(label_id) if label_id <= 0 => return not_found("label"),
        label_id => label_id,
    };
    if let Some(label_id) = label_id {
        match hail_db::labels::get_label(&state.db, user.id, label_id).await {
            Ok(_) => {}
            Err(hail_db::labels::LabelDbError::Sqlx(sqlx::Error::RowNotFound)) => {
                return not_found("label");
            }
            Err(err) => {
                tracing::error!(user_id = user.id, label_id, error = %err, "search label lookup failed");
                return internal();
            }
        }
    }

    let mut results = Vec::new();

    if scope.includes_mail() {
        let mut mail = match provider
            .search(&state, user.jmap_token.clone(), q, mailbox, SEARCH_LIMIT)
            .await
        {
            Ok(mail) => mail,
            Err(err) => {
                tracing::warn!(user_id = user.id, error = %err.0, "mail search failed");
                return internal();
            }
        };
        if let Some(label_id) = label_id {
            mail = match filter_search_by_label(&state, user.id, label_id, mail).await {
                Ok(mail) => mail,
                Err(err) => {
                    tracing::error!(user_id = user.id, label_id, error = %err, "search label filter lookup failed");
                    return internal();
                }
            };
        }
        if let Err(err) = annotate_search_labels(&state, user.id, &mut mail).await {
            tracing::error!(user_id = user.id, error = %err, "search result label lookup failed");
            return internal();
        }
        results.extend(mail.into_iter().map(SearchResult::from));
    }

    if scope.includes_notes() && label_id.is_none() {
        let notes = match search_notes(&state, user.id, q).await {
            Ok(notes) => notes,
            Err(err) => {
                tracing::error!(user_id = user.id, error = %err, "contact notes search failed");
                return internal();
            }
        };
        results.extend(notes);
    }

    Json(SearchResponse { results }).into_response()
}

async fn search_notes(
    state: &AppState,
    user_id: i64,
    q: &str,
) -> Result<Vec<SearchResult>, sqlx::Error> {
    let pattern = format!("%{}%", escape_like(q));
    let rows = sqlx::query_as::<_, (String, String, DateTime<Utc>)>(
        "SELECT address, markdown, updated_at \
         FROM contact_notes \
         WHERE user_id = ?1 AND markdown LIKE ?2 ESCAPE '\\' \
         ORDER BY updated_at DESC, address ASC \
         LIMIT ?3",
    )
    .bind(user_id)
    .bind(pattern)
    .bind(SEARCH_LIMIT as i64)
    .fetch_all(&state.db)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(address, markdown, updated_at)| SearchResult::ContactNote {
                address,
                markdown,
                updated_at,
            },
        )
        .collect())
}

async fn list_bubble_ups(
    state: &AppState,
    user_id: i64,
) -> Result<Vec<BubbleUpViewItem>, sqlx::Error> {
    sqlx::query_as::<_, (i64, String, DateTime<Utc>, DateTime<Utc>)>(
        "SELECT id, thread_id, surface_at, created_at \
         FROM bubble_ups \
         WHERE user_id = ?1 AND datetime(surface_at) > datetime('now') AND fired_at IS NULL \
         ORDER BY surface_at ASC",
    )
    .bind(user_id)
    .fetch_all(&state.db)
    .await
    .map(|rows| {
        rows.into_iter()
            .map(
                |(bubble_id, thread_id, surface_at, created_at)| BubbleUpViewItem {
                    bubble_id,
                    thread_id,
                    surface_at,
                    created_at,
                    from: String::new(),
                    subject: String::new(),
                    preview: String::new(),
                },
            )
            .collect()
    })
}

async fn load_view_counts(
    state: &AppState,
    user: &AuthUser,
    provider: Arc<dyn MailViewProvider>,
) -> Result<ViewCountsResponse, String> {
    let imbox_page = provider
        .list(
            state,
            user.jmap_token.clone(),
            MailView::Imbox,
            None,
            MAX_LIMIT,
            MailViewListOpts::default(),
        )
        .await
        .map_err(|err| err.0)?;
    let imbox_new = count_imbox_new(state, user.id, imbox_page.items).await?;
    let feed_unread = provider
        .count(state, user.jmap_token.clone(), MailView::Feed, true)
        .await
        .map_err(|err| err.0)?;
    let papertrail_unread = provider
        .count(state, user.jmap_token.clone(), MailView::Papertrail, true)
        .await
        .map_err(|err| err.0)?;
    let drafts = provider
        .count(state, user.jmap_token.clone(), MailView::Drafts, false)
        .await
        .map_err(|err| err.0)?;
    let spam = provider
        .count(state, user.jmap_token.clone(), MailView::Spam, false)
        .await
        .map_err(|err| err.0)?;
    let trash = provider
        .count(state, user.jmap_token.clone(), MailView::Trash, false)
        .await
        .map_err(|err| err.0)?;
    let screener_pending = scalar_count(
        state,
        user.id,
        "SELECT COUNT(*) FROM screener_rules WHERE user_id = ?1 AND decision = 'pending'",
    )
    .await?;
    let scheduled = scalar_count(
        state,
        user.id,
        "SELECT COUNT(*) FROM scheduled_sends WHERE user_id = ?1 AND status = 'pending'",
    )
    .await?;
    let set_aside = stack_count(state, user.id, "set_aside").await?;
    let reply_later = stack_count(state, user.id, "reply_later").await?;
    let bubble_up = scalar_count(
        state,
        user.id,
        "SELECT COUNT(*) FROM bubble_ups WHERE user_id = ?1 AND datetime(surface_at) > datetime('now') AND fired_at IS NULL",
    )
    .await?;

    Ok(ViewCountsResponse {
        imbox_new,
        feed_unread,
        papertrail_unread,
        screener_pending,
        drafts,
        scheduled,
        set_aside,
        reply_later,
        bubble_up,
        spam,
        trash,
    })
}

async fn count_imbox_new(
    state: &AppState,
    user_id: i64,
    items: Vec<MailViewItem>,
) -> Result<usize, String> {
    let seen_thread_ids = hail_db::seen_thread_ids(&state.db, user_id)
        .await
        .map_err(|err| err.to_string())?;
    let fired_bubble_up_thread_ids = hail_db::fired_bubble_up_thread_ids(&state.db, user_id)
        .await
        .map_err(|err| err.to_string())?;

    Ok(items
        .into_iter()
        .filter(|item| {
            item.unread
                && !seen_thread_ids.contains(&item.thread_id)
                && !fired_bubble_up_thread_ids.contains(&item.thread_id)
        })
        .count())
}

async fn stack_count(state: &AppState, user_id: i64, stack: &str) -> Result<usize, String> {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM stack_positions WHERE user_id = ?1 AND stack = ?2",
    )
    .bind(user_id)
    .bind(stack)
    .fetch_one(&state.db)
    .await
    .map(|count| count as usize)
    .map_err(|err| err.to_string())
}

async fn scalar_count(state: &AppState, user_id: i64, sql: &'static str) -> Result<usize, String> {
    sqlx::query_scalar::<_, i64>(sql)
        .bind(user_id)
        .fetch_one(&state.db)
        .await
        .map(|count| count as usize)
        .map_err(|err| err.to_string())
}

fn escape_like(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '%' | '_' | '\\' => {
                escaped.push('\\');
                escaped.push(ch);
            }
            _ => escaped.push(ch),
        }
    }
    escaped
}

async fn get_view(
    state: AppState,
    user: AuthUser,
    provider: Arc<dyn MailViewProvider>,
    query: ViewQuery,
    view: MailView,
) -> Response {
    let limit = query.normalized_limit();

    let page = match provider
        .list(
            &state,
            user.jmap_token.clone(),
            view,
            query.cursor,
            limit,
            MailViewListOpts {
                feed_render: FeedRenderOpts {
                    allow_remote_images: view == MailView::Feed
                        && crate::routes::users::load_user_prefs(&state.db, user.id)
                            .await
                            .map(|prefs| prefs.feed_load_remote_images)
                            .unwrap_or(false),
                },
            },
        )
        .await
    {
        Ok(items) => items,
        Err(err) => {
            tracing::warn!(user_id = user.id, view = ?view, error = %err.0, "mail view lookup failed");
            return internal();
        }
    };

    let mut items = page.items;

    if let Err(err) = annotate_note_flags(&state, user.id, &mut items).await {
        tracing::error!(user_id = user.id, view = ?view, error = %err, "thread note flag lookup failed");
        return internal();
    }
    if let Err(err) = annotate_item_labels(&state, user.id, &mut items).await {
        tracing::error!(user_id = user.id, view = ?view, error = %err, "thread label lookup failed");
        return internal();
    }

    Json(MailViewResponse {
        items,
        next_cursor: page.next_cursor,
    })
    .into_response()
}

async fn get_imbox_sectioned_view(
    state: AppState,
    user: AuthUser,
    provider: Arc<dyn MailViewProvider>,
    query: ViewQuery,
) -> Response {
    match get_unread_sectioned_view(
        &state,
        &user,
        provider,
        query,
        MailView::Imbox,
        true,
        true,
        false,
        25,
        None,
    )
    .await
    {
        Ok(sectioned) => Json(ImboxSectionedResponse {
            bubbled_up: sectioned.response.bubble_up.unwrap_or_default(),
            new_count: sectioned.response.new.len(),
            new_for_you: sectioned.response.new,
            previously_seen: sectioned.response.seen,
            previously_seen_total: sectioned.seen_total,
            next_cursor: sectioned.response.next_cursor,
        })
        .into_response(),
        Err(SectionedViewError::Provider(err)) => {
            tracing::warn!(user_id = user.id, error = %err.0, "imbox sectioned view lookup failed");
            internal()
        }
        Err(SectionedViewError::Sqlx(err)) => {
            tracing::error!(user_id = user.id, error = %err, "imbox sectioned metadata lookup failed");
            internal()
        }
        Err(SectionedViewError::Labels(err)) => {
            tracing::error!(user_id = user.id, error = %err, "imbox sectioned label lookup failed");
            internal()
        }
    }
}

#[derive(Debug)]
enum SectionedViewError {
    Provider(MailViewError),
    Sqlx(sqlx::Error),
    Labels(hail_db::labels::LabelDbError),
}

impl From<MailViewError> for SectionedViewError {
    fn from(value: MailViewError) -> Self {
        Self::Provider(value)
    }
}

impl From<sqlx::Error> for SectionedViewError {
    fn from(value: sqlx::Error) -> Self {
        Self::Sqlx(value)
    }
}

impl From<hail_db::labels::LabelDbError> for SectionedViewError {
    fn from(value: hail_db::labels::LabelDbError) -> Self {
        Self::Labels(value)
    }
}

async fn get_unread_sectioned_view(
    state: &AppState,
    user: &AuthUser,
    provider: Arc<dyn MailViewProvider>,
    query: ViewQuery,
    view: MailView,
    include_bubble_up: bool,
    use_thread_seen_state: bool,
    overfetch_seen: bool,
    seen_limit: usize,
    seen_cursor: Option<usize>,
) -> Result<LoadedSectionedMailView, SectionedViewError> {
    let limit = query.normalized_limit();
    let seen_offset = query
        .cursor
        .as_deref()
        .and_then(|cursor| cursor.parse::<usize>().ok())
        .or(seen_cursor)
        .unwrap_or(0);

    let fetch_limit = if overfetch_seen {
        limit
            .saturating_add(seen_offset)
            .saturating_add(seen_limit)
            .saturating_add(1)
    } else {
        limit
    };

    let mut page = provider
        .list(
            state,
            user.jmap_token.clone(),
            view,
            None,
            fetch_limit,
            MailViewListOpts::default(),
        )
        .await?;
    let mut items = std::mem::take(&mut page.items);

    annotate_note_flags(state, user.id, &mut items).await?;
    annotate_item_labels(state, user.id, &mut items).await?;

    let seen_thread_ids = if use_thread_seen_state {
        hail_db::seen_thread_ids(&state.db, user.id).await?
    } else {
        HashSet::new()
    };
    let fired_bubble_up_thread_ids = if include_bubble_up {
        hail_db::fired_bubble_up_thread_ids(&state.db, user.id).await?
    } else {
        HashSet::new()
    };

    let mut bubble_up = include_bubble_up.then(Vec::new);
    let mut new = Vec::new();
    let mut seen = Vec::new();
    let mut seen_total = 0;

    for item in items {
        if fired_bubble_up_thread_ids.contains(&item.thread_id) {
            if let Some(bubble_up) = &mut bubble_up {
                bubble_up.push(item);
            }
            continue;
        }

        let is_seen = if use_thread_seen_state {
            seen_thread_ids.contains(&item.thread_id) || item.unread_count == 0
        } else {
            item.unread_count == 0
        };

        if is_seen {
            if seen_total >= seen_offset && seen.len() < seen_limit {
                seen.push(item);
            }
            seen_total += 1;
        } else if new.len() < limit {
            new.push(item);
        }
    }

    let next_cursor =
        (seen_total > seen_offset + seen.len()).then(|| (seen_offset + seen.len()).to_string());

    Ok(LoadedSectionedMailView {
        response: SectionedMailViewResponse {
            bubble_up,
            new,
            seen,
            next_cursor,
        },
        seen_total,
    })
}

async fn annotate_note_flags(
    state: &AppState,
    user_id: i64,
    items: &mut [MailViewItem],
) -> Result<(), sqlx::Error> {
    let thread_ids: Vec<String> = items
        .iter()
        .map(|item| item.thread_id.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    if thread_ids.is_empty() {
        return Ok(());
    }

    let mut builder = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
        "SELECT DISTINCT thread_id FROM thread_notes WHERE user_id = ",
    );
    builder.push_bind(user_id);
    builder.push(" AND thread_id IN (");
    let mut separated = builder.separated(", ");
    for thread_id in &thread_ids {
        separated.push_bind(thread_id);
    }
    separated.push_unseparated(")");

    let note_thread_ids: HashSet<String> = builder
        .build_query_scalar()
        .fetch_all(&state.db)
        .await?
        .into_iter()
        .collect();

    for item in items {
        item.has_notes = note_thread_ids.contains(&item.thread_id);
    }

    Ok(())
}

async fn annotate_item_labels(
    state: &AppState,
    user_id: i64,
    items: &mut [MailViewItem],
) -> Result<(), hail_db::labels::LabelDbError> {
    let labels_by_thread_id = labels_by_thread_id(
        state,
        user_id,
        items.iter().map(|item| item.thread_id.as_str()),
    )
    .await?;
    for item in items {
        item.labels = labels_by_thread_id
            .get(&item.thread_id)
            .cloned()
            .unwrap_or_default();
    }
    Ok(())
}

async fn annotate_search_labels(
    state: &AppState,
    user_id: i64,
    items: &mut [MailSearchResult],
) -> Result<(), hail_db::labels::LabelDbError> {
    let labels_by_thread_id = labels_by_thread_id(
        state,
        user_id,
        items.iter().map(|item| item.thread_id.as_str()),
    )
    .await?;
    for item in items {
        item.labels = labels_by_thread_id
            .get(&item.thread_id)
            .cloned()
            .unwrap_or_default();
    }
    Ok(())
}

async fn filter_search_by_label(
    state: &AppState,
    user_id: i64,
    label_id: i64,
    items: Vec<MailSearchResult>,
) -> Result<Vec<MailSearchResult>, hail_db::labels::LabelDbError> {
    let thread_ids: Vec<String> = items
        .iter()
        .map(|item| item.thread_id.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let assigned_thread_ids =
        hail_db::labels::assigned_thread_ids_for_label(&state.db, user_id, label_id, &thread_ids)
            .await?
            .into_iter()
            .collect::<HashSet<_>>();
    Ok(items
        .into_iter()
        .filter(|item| assigned_thread_ids.contains(&item.thread_id))
        .collect())
}

async fn labels_by_thread_id<'a>(
    state: &AppState,
    user_id: i64,
    thread_ids: impl Iterator<Item = &'a str>,
) -> Result<HashMap<String, Vec<LabelResponse>>, hail_db::labels::LabelDbError> {
    let thread_ids: Vec<String> = thread_ids
        .map(ToOwned::to_owned)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let labels_by_thread_id =
        hail_db::labels::list_labels_for_threads(&state.db, user_id, &thread_ids).await?;
    Ok(labels_by_thread_id
        .into_iter()
        .map(|(thread_id, labels)| {
            (
                thread_id,
                labels.into_iter().map(LabelResponse::from).collect(),
            )
        })
        .collect())
}
