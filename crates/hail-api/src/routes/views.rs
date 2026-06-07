//! Mail list and unified search endpoints.
//!
//! Imbox, Feed, and Paper Trail are sourced from Stalwart/JMAP by querying
//! hail-owned classification keywords (`$hail_imbox`, `$hail_feed`,
//! `$hail_papertrail`). Drafts are sourced from the user's JMAP Drafts mailbox
//! or `$draft` keyword. Unified search combines JMAP mail text search with
//! hail-owned contact notes stored in SQLite.
//! `cursor` is an opaque thread cursor. Mail views are collapsed by JMAP
//! `threadId`; pagination advances by rendered conversations, not raw emails.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use axum::extract::{Extension, Query, State};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use chrono::{DateTime, TimeZone, Utc};
use hail_core::mail_render::{
    FeedRenderOpts, plaintext_body_to_html, render_feed_html, strip_quoted_history,
};
use hail_core::{HAIL_SPAM_KEYWORD, MailClassification, SPAM_KEYWORD};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::{OpenApiRouter, UtoipaMethodRouterExt};
use utoipa_axum::routes;

use crate::middleware::auth::AuthUser;
use crate::routes::jmap_helpers::{
    MAIL_VIEW_PROPERTIES, hydrate_thread_previews, jmap_session, preview_from_email,
    trash_mailbox_id,
};
use crate::routes::labels::LabelResponse;
use crate::routes::response::{bad_request, internal, not_found};
use crate::routes::threads::MailboxRole;
use crate::state::AppState;

const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 100;
const SEARCH_LIMIT: usize = 50;
const FEED_EMAIL_GET_CHUNK_SIZE: usize = 10;
const FEED_MAX_BODY_VALUE_BYTES: usize = 64 * 1024;

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

pub struct JmapMailViewProvider;

impl MailViewProvider for JmapMailViewProvider {
    fn list<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        view: MailView,
        cursor: Option<String>,
        limit: usize,
        opts: MailViewListOpts,
    ) -> Pin<Box<dyn Future<Output = Result<MailViewPage, MailViewError>> + Send + 'a>> {
        Box::pin(async move {
            use hail_jmap::jmap_client::email::query as email_query;

            let session = jmap_session(state, token)
                .await
                .map_err(MailViewError::provider)?;
            let Some(filter) = mail_view_filter(&session, view).await? else {
                return Ok(MailViewPage::empty());
            };

            let ids = query_all_mail_view_ids(
                &session,
                filter,
                Some(vec![email_query::Comparator::received_at().descending()]),
            )
            .await?;
            if ids.is_empty() {
                return Ok(MailViewPage::empty());
            }

            let emails = fetch_mail_view_emails(&session, ids, view).await?;
            let grouped = group_mail_view_emails(emails, view, opts.feed_render)?;
            Ok(page_mail_view_items(grouped, cursor.as_deref(), limit))
        })
    }

    fn count<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        view: MailView,
        unread_only: bool,
    ) -> Pin<Box<dyn Future<Output = Result<usize, MailViewError>> + Send + 'a>> {
        Box::pin(async move {
            use hail_jmap::jmap_client::core::query::Filter;
            use hail_jmap::jmap_client::email::query as email_query;

            let session = jmap_session(state, token)
                .await
                .map_err(MailViewError::provider)?;
            let Some(filter) = mail_view_filter(&session, view).await? else {
                return Ok(0);
            };

            if unread_only {
                let filter = Filter::and([
                    filter,
                    Filter::not([email_query::Filter::has_keyword("$seen".to_string())]),
                ]);
                let ids = query_all_mail_view_ids(&session, filter, None).await?;
                if ids.is_empty() {
                    return Ok(0);
                }
                let emails = fetch_mail_view_emails(&session, ids, view).await?;
                let thread_ids = emails
                    .into_iter()
                    .filter_map(|email| email.thread_id().map(ToOwned::to_owned))
                    .collect::<HashSet<_>>();
                return Ok(thread_ids.len());
            }

            let mut request = session.client().build();
            request
                .query_email()
                .filter(filter)
                .limit(0)
                .calculate_total(true);
            let query = request
                .send_query_email()
                .await
                .map_err(|err| MailViewError::provider(err.to_string()))?;
            query.total().ok_or_else(|| {
                MailViewError::provider(format!("JMAP Email/query omitted total for {view:?}"))
            })
        })
    }
}

async fn mail_view_filter(
    session: &hail_jmap::Session,
    view: MailView,
) -> Result<
    Option<
        hail_jmap::jmap_client::core::query::Filter<hail_jmap::jmap_client::email::query::Filter>,
    >,
    MailViewError,
> {
    use hail_jmap::jmap_client::core::query::Filter;
    use hail_jmap::jmap_client::email::query as email_query;

    let mut filter = Filter::from(email_query::Filter::has_keyword(view.keyword()));
    if view == MailView::Spam {
        let junk_mailbox_id =
            hail_jmap::mailbox_id_by_role(session, hail_jmap::jmap_client::mailbox::Role::Junk)
                .await
                .map_err(|err| MailViewError::provider(err.to_string()))?;
        let mut conditions = vec![
            Filter::from(email_query::Filter::has_keyword(SPAM_KEYWORD.to_string())),
            Filter::from(email_query::Filter::has_keyword(
                HAIL_SPAM_KEYWORD.to_string(),
            )),
        ];
        if let Some(junk_mailbox_id) = junk_mailbox_id {
            conditions.push(Filter::from(email_query::Filter::in_mailbox(
                junk_mailbox_id,
            )));
        }
        filter = Filter::or(conditions);
    } else if view == MailView::Drafts {
        if let Some(drafts_mailbox_id) =
            hail_jmap::mailbox_id_by_role(session, hail_jmap::jmap_client::mailbox::Role::Drafts)
                .await
                .map_err(|err| MailViewError::provider(err.to_string()))?
        {
            filter = Filter::from(email_query::Filter::in_mailbox(drafts_mailbox_id));
        }
    } else if view == MailView::Trash {
        let Some(trash_mailbox_id) = trash_mailbox_id(session)
            .await
            .map_err(MailViewError::provider)?
        else {
            return Ok(None);
        };
        filter = Filter::from(email_query::Filter::in_mailbox(trash_mailbox_id));
    } else if view == MailView::Archive {
        let Some(archive_mailbox_id) =
            hail_jmap::mailbox_id_by_role(session, MailboxRole::Archive.jmap())
                .await
                .map_err(|err| MailViewError::provider(err.to_string()))?
        else {
            return Ok(None);
        };
        filter = Filter::from(email_query::Filter::in_mailbox(archive_mailbox_id));
    }

    Ok(Some(filter))
}

async fn fetch_mail_view_emails(
    session: &hail_jmap::Session,
    ids: Vec<String>,
    view: MailView,
) -> Result<Vec<hail_jmap::jmap_client::email::Email>, MailViewError> {
    if view != MailView::Feed {
        let mut request = session.client().build();
        request
            .get_email()
            .ids(ids)
            .properties(MAIL_VIEW_PROPERTIES.iter().cloned());
        let mut response = request
            .send_get_email()
            .await
            .map_err(|err| MailViewError::provider(err.to_string()))?;
        return Ok(response.take_list());
    }

    let mut emails = Vec::with_capacity(ids.len());
    for chunk in ids.chunks(FEED_EMAIL_GET_CHUNK_SIZE) {
        let mut request = session.client().build();
        let get_email = request.get_email();
        get_email
            .ids(chunk.to_vec())
            .properties(MAIL_VIEW_PROPERTIES.iter().cloned());
        get_email
            .arguments()
            .fetch_html_body_values(true)
            .fetch_text_body_values(true)
            .max_body_value_bytes(FEED_MAX_BODY_VALUE_BYTES);
        let mut response = request
            .send_get_email()
            .await
            .map_err(|err| MailViewError::provider(err.to_string()))?;
        emails.extend(response.take_list());
    }
    Ok(emails)
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

impl MailViewPage {
    fn empty() -> Self {
        Self {
            items: Vec::new(),
            next_cursor: None,
        }
    }
}

#[derive(Debug)]
struct GroupedMailViewThread {
    item: MailViewItem,
    received_at_sort: i64,
}

type EmailQueryFilter =
    hail_jmap::jmap_client::core::query::Filter<hail_jmap::jmap_client::email::query::Filter>;
type EmailQueryComparator = hail_jmap::jmap_client::core::query::Comparator<
    hail_jmap::jmap_client::email::query::Comparator,
>;

async fn query_all_mail_view_ids(
    session: &hail_jmap::Session,
    filter: EmailQueryFilter,
    sort: Option<Vec<EmailQueryComparator>>,
) -> Result<Vec<String>, MailViewError> {
    const QUERY_CHUNK_SIZE: usize = 256;

    let mut position = 0;
    let mut ids = Vec::new();
    loop {
        let mut request = session.client().build();
        let query = request.query_email();
        query
            .filter(filter.clone())
            .position(position)
            .limit(QUERY_CHUNK_SIZE);
        if let Some(sort) = sort.as_ref() {
            query.sort(sort.clone());
        }
        let mut response = request
            .send_query_email()
            .await
            .map_err(|err| MailViewError::provider(err.to_string()))?;
        let chunk = response.take_ids();
        let chunk_len = chunk.len();
        ids.extend(chunk);
        if chunk_len < QUERY_CHUNK_SIZE {
            break;
        }
        position += i32::try_from(chunk_len)
            .map_err(|_| MailViewError::provider("Email/query position overflow"))?;
    }

    Ok(ids)
}

fn group_mail_view_emails(
    emails: Vec<hail_jmap::jmap_client::email::Email>,
    view: MailView,
    feed_render_opts: FeedRenderOpts,
) -> Result<Vec<GroupedMailViewThread>, MailViewError> {
    let classification = view.classification();
    let mut by_thread: HashMap<String, (usize, usize, hail_jmap::jmap_client::email::Email)> =
        HashMap::new();

    for email in emails {
        let thread_id = required_jmap_field(email.thread_id(), "threadId")
            .map_err(|err| MailViewError::malformed_email(err.field))?;
        let unread = !email.keywords().into_iter().any(|kw| kw == "$seen");
        let received_at_sort = email.received_at().unwrap_or(0);
        match by_thread.get_mut(&thread_id) {
            Some((message_count, unread_count, newest)) => {
                *message_count += 1;
                if unread {
                    *unread_count += 1;
                }
                let newest_received_at = newest.received_at().unwrap_or(0);
                let newest_id = newest.id().unwrap_or_default();
                let email_id = email.id().unwrap_or_default();
                if received_at_sort > newest_received_at
                    || (received_at_sort == newest_received_at && email_id > newest_id)
                {
                    *newest = email;
                }
            }
            None => {
                by_thread.insert(thread_id, (1, usize::from(unread), email));
            }
        }
    }

    let mut grouped = by_thread
        .into_iter()
        .map(|(thread_id, (message_count, unread_count, email))| {
            let email_id = required_jmap_field(email.id(), "id")
                .map_err(|err| MailViewError::malformed_email(err.field))?;
            let feed_render =
                (view == MailView::Feed).then(|| render_feed_body(&email, feed_render_opts));
            let (feed_html, feed_html_with_images, feed_blocked_trackers, feed_blocked_images) =
                match feed_render {
                    Some(render) => (
                        Some(render.html),
                        Some(render.html_with_remote_images),
                        Some(render.blocked_trackers),
                        Some(render.blocked_images),
                    ),
                    None => (None, None, None, None),
                };
            let received_at_sort = email.received_at().unwrap_or(0);

            Ok(GroupedMailViewThread {
                received_at_sort,
                item: MailViewItem {
                    thread_id,
                    email_id,
                    from: format_from(email.from()),
                    to: format_addresses(email.to()),
                    cc: format_addresses(email.cc()),
                    bcc: format_addresses(email.bcc()),
                    subject: email.subject().unwrap_or_default().to_string(),
                    preview: preview_from_email(&email),
                    received_at: email
                        .received_at()
                        .and_then(|ts| Utc.timestamp_opt(ts, 0).single()),
                    unread: unread_count > 0,
                    message_count,
                    unread_count,
                    classification,
                    has_notes: false,
                    labels: Vec::new(),
                    feed_html,
                    feed_html_with_images,
                    feed_blocked_trackers,
                    feed_blocked_images,
                },
            })
        })
        .collect::<Result<Vec<_>, MailViewError>>()?;

    grouped.sort_by(|left, right| {
        right
            .received_at_sort
            .cmp(&left.received_at_sort)
            .then_with(|| left.item.thread_id.cmp(&right.item.thread_id))
    });
    Ok(grouped)
}

fn page_mail_view_items(
    grouped: Vec<GroupedMailViewThread>,
    cursor: Option<&str>,
    limit: usize,
) -> MailViewPage {
    let offset = cursor
        .and_then(decode_thread_cursor)
        .and_then(|cursor| {
            grouped
                .iter()
                .position(|thread| {
                    thread.item.thread_id == cursor.thread_id
                        && thread.received_at_sort == cursor.received_at_sort
                })
                .map(|index| index + 1)
        })
        .unwrap_or(0);

    let mut items = grouped
        .into_iter()
        .skip(offset)
        .take(limit.saturating_add(1))
        .collect::<Vec<_>>();
    let has_more = items.len() > limit;
    if has_more {
        items.truncate(limit);
    }
    let next_cursor = has_more
        .then(|| items.last().map(|thread| encode_thread_cursor(thread)))
        .flatten();

    MailViewPage {
        items: items.into_iter().map(|thread| thread.item).collect(),
        next_cursor,
    }
}

#[derive(Debug)]
struct ThreadCursor {
    thread_id: String,
    received_at_sort: i64,
}

fn encode_thread_cursor(thread: &GroupedMailViewThread) -> String {
    format!("t:{}:{}", thread.received_at_sort, thread.item.thread_id)
}

fn decode_thread_cursor(value: &str) -> Option<ThreadCursor> {
    let rest = value.strip_prefix("t:")?;
    let (received_at, thread_id) = rest.split_once(':')?;
    Some(ThreadCursor {
        thread_id: thread_id.to_string(),
        received_at_sort: received_at.parse().ok()?,
    })
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

            let ids = {
                let mut request = session.client().build();
                request
                    .query_email()
                    .filter(filter)
                    .sort([email_query::Comparator::received_at().descending()])
                    .limit(limit.saturating_mul(5).max(limit));
                let mut query = request
                    .send_query_email()
                    .await
                    .map_err(|err| SearchError::provider(err.to_string()))?;
                query.take_ids()
            };
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

            let grouped = group_mail_view_emails(
                response.take_list(),
                MailView::Imbox,
                FeedRenderOpts::default(),
            )
            .map_err(|err| SearchError::provider(err.0))?;
            Ok(grouped
                .into_iter()
                .take(limit)
                .map(|thread| {
                    let item = thread.item;
                    MailSearchResult {
                        thread_id: item.thread_id,
                        email_id: item.email_id,
                        from: item.from,
                        subject: item.subject,
                        preview: item.preview,
                        message_count: item.message_count,
                        unread_count: item.unread_count,
                        unread: item.unread,
                        received_at: item.received_at,
                        labels: Vec::new(),
                    }
                })
                .collect())
        })
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

fn format_from(from: Option<&[hail_jmap::jmap_client::email::EmailAddress]>) -> String {
    from.and_then(|addresses| addresses.first())
        .map(format_address)
        .unwrap_or_default()
}

struct RenderedFeedBody {
    html: String,
    html_with_remote_images: String,
    blocked_trackers: Vec<FeedBlockedTrackerResponse>,
    blocked_images: usize,
}

fn render_feed_body(
    email: &hail_jmap::jmap_client::email::Email,
    opts: FeedRenderOpts,
) -> RenderedFeedBody {
    let body_html = if let Some(html_body) =
        body_from_parts(email, email.html_body()).filter(|body| !body.trim().is_empty())
    {
        html_body
    } else {
        let text = body_from_parts(email, email.text_body()).unwrap_or_default();
        plaintext_body_to_html(&text)
    };
    let stripped = strip_quoted_history(&body_html);
    let sanitized = render_feed_html(&stripped.html, opts);

    RenderedFeedBody {
        html: sanitized.html,
        html_with_remote_images: sanitized.html_with_remote_images,
        blocked_trackers: sanitized
            .blocked_trackers
            .into_iter()
            .map(|tracker| FeedBlockedTrackerResponse {
                src: tracker.src,
                reason: tracker.reason,
            })
            .collect(),
        blocked_images: sanitized.blocked_images,
    }
}

fn body_from_parts(
    email: &hail_jmap::jmap_client::email::Email,
    parts: Option<&[hail_jmap::jmap_client::email::EmailBodyPart]>,
) -> Option<String> {
    let mut body = String::new();
    for part in parts? {
        let Some(part_id) = part.part_id() else {
            continue;
        };
        let Some(value) = email.body_value(part_id) else {
            continue;
        };
        body.push_str(value.value());
    }
    Some(body)
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
