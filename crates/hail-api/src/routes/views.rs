//! Mail list and unified search endpoints.
//!
//! Imbox, Feed, and Paper Trail are sourced from Stalwart/JMAP by querying
//! hail-owned classification keywords (`$hail_imbox`, `$hail_feed`,
//! `$hail_papertrail`). Unified search combines JMAP mail text search with
//! hail-owned contact notes stored in SQLite.
//! `cursor` is accepted for API compatibility but intentionally ignored for v1;
//! responses always return `next_cursor: null`.

use std::fmt;
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
const SEARCH_LIMIT: usize = 50;

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
            use hail_jmap::jmap_client::email::Property;
            use hail_jmap::jmap_client::email::query as email_query;

            let session = hail_jmap::login_bearer(&state.config.stalwart.jmap_url, token)
                .await
                .map_err(|err| MailViewError::provider(err.to_string()))?;

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
                .map_err(|err| MailViewError::provider(err.to_string()))?;
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
                        subject: email.subject().unwrap_or_default().to_string(),
                        preview: email.preview().unwrap_or_default().to_string(),
                        received_at: email
                            .received_at()
                            .and_then(|ts| Utc.timestamp_opt(ts, 0).single()),
                        unread: !email.keywords().into_iter().any(|kw| kw == "$seen"),
                        classification,
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
        limit: usize,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<MailSearchResult>, SearchError>> + Send + 'a>> {
        Box::pin(async move {
            use hail_jmap::jmap_client::core::query::Filter;
            use hail_jmap::jmap_client::email::Property;
            use hail_jmap::jmap_client::email::query as email_query;

            let session = hail_jmap::login_bearer(&state.config.stalwart.jmap_url, token)
                .await
                .map_err(|err| SearchError::provider(err.to_string()))?;

            let mut request = session.client().build();
            request
                .query_email()
                .filter(Filter::from(email_query::Filter::text(q.to_string())))
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

            let props = [
                Property::Id,
                Property::ThreadId,
                Property::From,
                Property::Subject,
                Property::Preview,
                Property::ReceivedAt,
            ];
            let mut request = session.client().build();
            request.get_email().ids(ids).properties(props);
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
    router_with_providers(Arc::new(JmapMailViewProvider), Arc::new(JmapSearchProvider))
}

pub fn router_with_provider<P>(provider: Arc<P>) -> Router<AppState>
where
    P: MailViewProvider,
{
    router_with_providers(provider, Arc::new(EmptySearchProvider))
}

pub fn router_with_providers<P, S>(
    mail_provider: Arc<P>,
    search_provider: Arc<S>,
) -> Router<AppState>
where
    P: MailViewProvider,
    S: SearchProvider,
{
    Router::new()
        .route("/api/views/imbox", axum::routing::get(get_imbox::<P>))
        .route("/api/views/feed", axum::routing::get(get_feed::<P>))
        .route(
            "/api/views/papertrail",
            axum::routing::get(get_papertrail::<P>),
        )
        .route("/api/views/search", axum::routing::get(get_search::<S>))
        .layer(Extension(mail_provider))
        .layer(Extension(search_provider))
}

struct EmptySearchProvider;

impl SearchProvider for EmptySearchProvider {
    fn search<'a>(
        &'a self,
        _state: &'a AppState,
        _token: SecretString,
        _q: &'a str,
        _limit: usize,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<MailSearchResult>, SearchError>> + Send + 'a>> {
        Box::pin(async { Ok(Vec::new()) })
    }
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

#[derive(Debug, Clone, Serialize)]
pub struct MailSearchResult {
    pub thread_id: String,
    pub email_id: String,
    pub from: String,
    pub subject: String,
    pub preview: String,
    pub received_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum SearchResult {
    Mail {
        thread_id: String,
        email_id: String,
        from: String,
        subject: String,
        preview: String,
        received_at: Option<DateTime<Utc>>,
    },
    ContactNote {
        address: String,
        markdown: String,
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

#[derive(Debug, Deserialize)]
struct ViewQuery {
    cursor: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct SearchQuery {
    q: Option<String>,
    scope: Option<String>,
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

#[derive(Debug, Serialize)]
struct MailViewResponse {
    items: Vec<MailViewItem>,
    next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
struct SearchResponse {
    results: Vec<SearchResult>,
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

async fn get_search<S>(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(provider): Extension<Arc<S>>,
    Query(query): Query<SearchQuery>,
) -> Response
where
    S: SearchProvider,
{
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

    let mut results = Vec::new();

    if scope.includes_mail() {
        let mail = match provider
            .search(&state, user.jmap_token.clone(), q, SEARCH_LIMIT)
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
