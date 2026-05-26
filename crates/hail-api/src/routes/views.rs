//! Mail list and unified search endpoints.
//!
//! Imbox, Feed, and Paper Trail are sourced from Stalwart/JMAP by querying
//! hail-owned classification keywords (`$hail_imbox`, `$hail_feed`,
//! `$hail_papertrail`). Drafts are sourced from the user's JMAP Drafts mailbox
//! or `$draft` keyword. Unified search combines JMAP mail text search with
//! hail-owned contact notes stored in SQLite.
//! `cursor` is accepted for API compatibility but intentionally ignored for v1;
//! responses always return `next_cursor: null`.

use std::collections::HashSet;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use axum::extract::{Extension, Query, State};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use chrono::{DateTime, TimeZone, Utc};
use hail_core::MailClassification;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::{OpenApiRouter, UtoipaMethodRouterExt};
use utoipa_axum::routes;

use crate::middleware::auth::AuthUser;
use crate::routes::jmap_helpers::{
    MAIL_VIEW_PROPERTIES, hydrate_thread_previews, jmap_session, trash_mailbox_id,
};
use crate::routes::response::{bad_request, internal};
use crate::routes::threads::MailboxRole;
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
        limit: usize,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<MailViewItem>, MailViewError>> + Send + 'a>>;
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
            use hail_jmap::jmap_client::email::query as email_query;

            let session = jmap_session(state, token)
                .await
                .map_err(MailViewError::provider)?;

            let mut request = session.client().build();
            let mut filter = Filter::from(email_query::Filter::has_keyword(view.keyword()));
            if view == MailView::Drafts {
                if let Some(drafts_mailbox_id) = hail_jmap::mailbox_id_by_role(
                    &session,
                    hail_jmap::jmap_client::mailbox::Role::Drafts,
                )
                .await
                .map_err(|err| MailViewError::provider(err.to_string()))?
                {
                    filter = Filter::from(email_query::Filter::in_mailbox(drafts_mailbox_id));
                }
            } else if view == MailView::Trash {
                let Some(trash_mailbox_id) = trash_mailbox_id(&session)
                    .await
                    .map_err(MailViewError::provider)?
                else {
                    return Ok(Vec::new());
                };
                filter = Filter::from(email_query::Filter::in_mailbox(trash_mailbox_id));
            } else if view == MailView::Archive {
                let Some(archive_mailbox_id) =
                    hail_jmap::mailbox_id_by_role(&session, MailboxRole::Archive.jmap())
                        .await
                        .map_err(|err| MailViewError::provider(err.to_string()))?
                else {
                    return Ok(Vec::new());
                };
                filter = Filter::from(email_query::Filter::in_mailbox(archive_mailbox_id));
            }
            request
                .query_email()
                .filter(filter)
                .sort([email_query::Comparator::received_at().descending()])
                .limit(limit);
            let mut query = request
                .send_query_email()
                .await
                .map_err(|err| MailViewError::provider(err.to_string()))?;
            let ids = query.take_ids();
            if ids.is_empty() {
                return Ok(Vec::new());
            }

            let mut request = session.client().build();
            request
                .get_email()
                .ids(ids)
                .properties(MAIL_VIEW_PROPERTIES.iter().cloned());
            let mut response = request
                .send_get_email()
                .await
                .map_err(|err| MailViewError::provider(err.to_string()))?;

            let classification = view.classification();
            response
                .take_list()
                .into_iter()
                .map(|email| {
                    let thread_id = required_jmap_field(email.thread_id(), "threadId")
                        .map_err(|err| MailViewError::malformed_email(err.field))?;
                    let email_id = required_jmap_field(email.id(), "id")
                        .map_err(|err| MailViewError::malformed_email(err.field))?;

                    Ok(MailViewItem {
                        thread_id,
                        email_id,
                        from: format_from(email.from()),
                        to: format_addresses(email.to()),
                        cc: format_addresses(email.cc()),
                        bcc: format_addresses(email.bcc()),
                        subject: email.subject().unwrap_or_default().to_string(),
                        preview: email.preview().unwrap_or_default().to_string(),
                        received_at: email
                            .received_at()
                            .and_then(|ts| Utc.timestamp_opt(ts, 0).single()),
                        unread: !email.keywords().into_iter().any(|kw| kw == "$seen"),
                        classification,
                        has_notes: false,
                    })
                })
                .collect()
        })
    }
}

#[derive(Debug)]
pub struct MailViewError(String);

impl MailViewError {
    pub fn provider(message: impl Into<String>) -> Self {
        Self(message.into())
    }

    fn malformed_email(field: &'static str) -> Self {
        Self(format!("JMAP Email missing required {field}"))
    }
}

#[derive(Debug)]
pub struct SearchError(String);

impl SearchError {
    pub fn provider(message: impl Into<String>) -> Self {
        Self(message.into())
    }

    fn malformed_email(field: &'static str) -> Self {
        Self(format!("JMAP Email missing required {field}"))
    }
}

#[derive(Debug)]
struct MissingJmapEmailField {
    field: &'static str,
}

fn required_jmap_field<T>(
    value: Option<T>,
    field: &'static str,
) -> Result<String, MissingJmapEmailField>
where
    T: fmt::Display,
{
    let Some(value) = value else {
        return Err(MissingJmapEmailField { field });
    };
    let value = value.to_string();
    if value.is_empty() {
        return Err(MissingJmapEmailField { field });
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::required_jmap_field;

    #[test]
    fn required_jmap_field_rejects_missing_and_empty_values() {
        assert_eq!(
            required_jmap_field::<&str>(None, "id").unwrap_err().field,
            "id"
        );
        assert_eq!(
            required_jmap_field(Some(""), "threadId").unwrap_err().field,
            "threadId"
        );
    }

    #[test]
    fn required_jmap_field_returns_non_empty_value() {
        assert_eq!(
            required_jmap_field(Some("email-1"), "id").unwrap(),
            "email-1"
        );
    }
}

pub struct JmapSearchProvider;

impl SearchProvider for JmapSearchProvider {
    fn search<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        q: &'a str,
        mailbox: Option<SearchMailbox>,
        limit: usize,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<MailSearchResult>, SearchError>> + Send + 'a>> {
        Box::pin(async move {
            use hail_jmap::jmap_client::core::query::Filter;
            use hail_jmap::jmap_client::email::query as email_query;

            let session = jmap_session(state, token)
                .await
                .map_err(SearchError::provider)?;

            let text_filter = Filter::from(email_query::Filter::text(q.to_string()));
            let mailbox_filter = match mailbox {
                Some(SearchMailbox::Imbox) => Some(Filter::from(email_query::Filter::has_keyword(
                    MailClassification::Imbox.keyword().to_string(),
                ))),
                Some(SearchMailbox::Feed) => Some(Filter::from(email_query::Filter::has_keyword(
                    MailClassification::Feed.keyword().to_string(),
                ))),
                Some(SearchMailbox::Papertrail) => {
                    Some(Filter::from(email_query::Filter::has_keyword(
                        MailClassification::Papertrail.keyword().to_string(),
                    )))
                }
                Some(SearchMailbox::Archive) => {
                    let Some(mailbox_id) = hail_jmap::mailbox_id_by_role(
                        &session,
                        hail_jmap::jmap_client::mailbox::Role::Archive,
                    )
                    .await
                    .map_err(|err| SearchError::provider(err.to_string()))?
                    else {
                        return Ok(Vec::new());
                    };
                    Some(Filter::from(email_query::Filter::in_mailbox(mailbox_id)))
                }
                Some(SearchMailbox::Trash) => {
                    let Some(mailbox_id) = hail_jmap::mailbox_id_by_role(
                        &session,
                        hail_jmap::jmap_client::mailbox::Role::Trash,
                    )
                    .await
                    .map_err(|err| SearchError::provider(err.to_string()))?
                    else {
                        return Ok(Vec::new());
                    };
                    Some(Filter::from(email_query::Filter::in_mailbox(mailbox_id)))
                }
                Some(SearchMailbox::Drafts) => {
                    let Some(mailbox_id) = hail_jmap::mailbox_id_by_role(
                        &session,
                        hail_jmap::jmap_client::mailbox::Role::Drafts,
                    )
                    .await
                    .map_err(|err| SearchError::provider(err.to_string()))?
                    else {
                        return Ok(Vec::new());
                    };
                    Some(Filter::from(email_query::Filter::in_mailbox(mailbox_id)))
                }
                None => None,
            };
            let filter = match mailbox_filter {
                Some(mailbox_filter) => Filter::and([text_filter, mailbox_filter]),
                None => text_filter,
            };

            let mut request = session.client().build();
            request
                .query_email()
                .filter(filter)
                .sort([email_query::Comparator::received_at().descending()])
                .limit(limit);
            let mut query = request
                .send_query_email()
                .await
                .map_err(|err| SearchError::provider(err.to_string()))?;
            let ids = query.take_ids();
            if ids.is_empty() {
                return Ok(Vec::new());
            }

            let mut request = session.client().build();
            request
                .get_email()
                .ids(ids)
                .properties(MAIL_VIEW_PROPERTIES.iter().cloned());
            let mut response = request
                .send_get_email()
                .await
                .map_err(|err| SearchError::provider(err.to_string()))?;

            response
                .take_list()
                .into_iter()
                .map(|email| {
                    let thread_id = required_jmap_field(email.thread_id(), "threadId")
                        .map_err(|err| SearchError::malformed_email(err.field))?;
                    let email_id = required_jmap_field(email.id(), "id")
                        .map_err(|err| SearchError::malformed_email(err.field))?;

                    Ok(MailSearchResult {
                        thread_id,
                        email_id,
                        from: format_from(email.from()),
                        subject: email.subject().unwrap_or_default().to_string(),
                        preview: email.preview().unwrap_or_default().to_string(),
                        received_at: email
                            .received_at()
                            .and_then(|ts| Utc.timestamp_opt(ts, 0).single()),
                    })
                })
                .collect()
        })
    }
}

pub fn router() -> Router<AppState> {
    Router::from(openapi_router_with_providers(
        Arc::new(JmapMailViewProvider),
        Arc::new(JmapSearchProvider),
    ))
}

/// Build the OpenAPI-tracked router for production mail views and search.
pub fn openapi_router() -> OpenApiRouter<AppState> {
    openapi_router_with_providers(Arc::new(JmapMailViewProvider), Arc::new(JmapSearchProvider))
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
        .routes(routes!(get_imbox).layer(Extension(mail_provider.clone())))
        .routes(routes!(get_imbox_sectioned).layer(Extension(mail_provider.clone())))
        .routes(routes!(get_feed).layer(Extension(mail_provider.clone())))
        .routes(routes!(get_papertrail).layer(Extension(mail_provider.clone())))
        .routes(routes!(get_drafts).layer(Extension(mail_provider.clone())))
        .routes(routes!(get_trash).layer(Extension(mail_provider.clone())))
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, ToSchema)]
pub enum MailView {
    Imbox,
    Feed,
    Papertrail,
    Drafts,
    Trash,
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
            Self::Archive => MailViewClassification::Archive,
        }
    }
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
    pub classification: MailViewClassification,
    pub has_notes: bool,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MailSearchResult {
    pub thread_id: String,
    pub email_id: String,
    pub from: String,
    pub subject: String,
    pub preview: String,
    #[schema(value_type = Option<String>, format = DateTime)]
    pub received_at: Option<DateTime<Utc>>,
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
        #[schema(value_type = Option<String>, format = DateTime)]
        received_at: Option<DateTime<Utc>>,
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
            received_at: item.received_at,
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

    let mut results = Vec::new();

    if scope.includes_mail() {
        let mail = match provider
            .search(&state, user.jmap_token.clone(), q, mailbox, SEARCH_LIMIT)
            .await
        {
            Ok(mail) => mail,
            Err(err) => {
                tracing::warn!(user_id = user.id, error = %err.0, "mail search failed");
                return internal();
            }
        };
        results.extend(mail.into_iter().map(SearchResult::from));
    }

    if scope.includes_notes() {
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
    // TODO(cursor): implement opaque JMAP anchor/queryState pagination. v1 only
    // accepts and ignores the cursor parameter, returning `next_cursor: null`.
    let _cursor = query.cursor.as_deref();
    let limit = query.normalized_limit();

    let mut items = match provider
        .list(&state, user.jmap_token.clone(), view, limit)
        .await
    {
        Ok(items) => items,
        Err(err) => {
            tracing::warn!(user_id = user.id, view = ?view, error = %err.0, "mail view lookup failed");
            return internal();
        }
    };

    if let Err(err) = annotate_note_flags(&state, user.id, &mut items).await {
        tracing::error!(user_id = user.id, view = ?view, error = %err, "thread note flag lookup failed");
        return internal();
    }

    Json(MailViewResponse {
        items,
        next_cursor: None,
    })
    .into_response()
}

async fn get_imbox_sectioned_view(
    state: AppState,
    user: AuthUser,
    provider: Arc<dyn MailViewProvider>,
    query: ViewQuery,
) -> Response {
    let _cursor = query.cursor.as_deref();
    let limit = query.normalized_limit();

    let mut items = match provider
        .list(&state, user.jmap_token.clone(), MailView::Imbox, limit)
        .await
    {
        Ok(items) => items,
        Err(err) => {
            tracing::warn!(user_id = user.id, error = %err.0, "imbox sectioned view lookup failed");
            return internal();
        }
    };

    if let Err(err) = annotate_note_flags(&state, user.id, &mut items).await {
        tracing::error!(user_id = user.id, error = %err, "imbox sectioned note flag lookup failed");
        return internal();
    }

    let seen_thread_ids = match hail_db::seen_thread_ids(&state.db, user.id).await {
        Ok(thread_ids) => thread_ids,
        Err(err) => {
            tracing::error!(user_id = user.id, error = %err, "seen thread id lookup failed");
            return internal();
        }
    };
    let fired_bubble_up_thread_ids = match hail_db::fired_bubble_up_thread_ids(&state.db, user.id)
        .await
    {
        Ok(thread_ids) => thread_ids,
        Err(err) => {
            tracing::error!(user_id = user.id, error = %err, "fired bubble-up thread id lookup failed");
            return internal();
        }
    };

    let mut bubbled_up = Vec::new();
    let mut new_for_you = Vec::new();
    let mut previously_seen = Vec::new();
    let mut previously_seen_total = 0;

    for item in items {
        if fired_bubble_up_thread_ids.contains(&item.thread_id) {
            bubbled_up.push(item);
        } else if seen_thread_ids.contains(&item.thread_id) {
            previously_seen_total += 1;
            if previously_seen.len() < 25 {
                previously_seen.push(item);
            }
        } else {
            new_for_you.push(item);
        }
    }

    let new_count = new_for_you.len();

    Json(ImboxSectionedResponse {
        bubbled_up,
        new_for_you,
        previously_seen,
        new_count,
        previously_seen_total,
    })
    .into_response()
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

fn format_from(from: Option<&[hail_jmap::jmap_client::email::EmailAddress]>) -> String {
    from.and_then(|addresses| addresses.first())
        .map(format_address)
        .unwrap_or_default()
}

fn format_addresses(
    addresses: Option<&[hail_jmap::jmap_client::email::EmailAddress]>,
) -> Vec<String> {
    addresses
        .unwrap_or_default()
        .iter()
        .map(format_address)
        .collect()
}

fn format_address(address: &hail_jmap::jmap_client::email::EmailAddress) -> String {
    match address.name() {
        Some(name) if !name.is_empty() => format!("{} <{}>", name, address.email()),
        _ => address.email().to_string(),
    }
}
