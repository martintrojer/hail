//! Worker-side bidirectional provider push loop for Gmail.
//!
//! The API layer records durable `provider_outbound_changes` when an operator
//! changes local read state, labels, or trash state and the provider account is
//! explicitly opted in. This module drains those rows into idempotent Gmail
//! `messages.batchModify` calls and marks rows applied only after Gmail accepts
//! the batch.

use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use async_trait::async_trait;
use hail_db::provider_audit_sanitizer::safe_provider_account_error_message;
use serde::Deserialize;
use sqlx::{Row, SqlitePool};
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::gmail_client::{BatchModifyMessagesRequest, GmailClient, GmailClientError, GmailTokenSource};

const MAX_BATCH_IDS: usize = 1_000;
const DEFAULT_IDLE_INTERVAL: Duration = Duration::from_secs(30);
const MAX_BACKOFF_SECS: i64 = 30 * 60;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ProviderOutboundChange {
    pub id: i64,
    pub provider_account_id: i64,
    pub jmap_email_id: String,
    pub provider_message_id: String,
    pub change_type: String,
    pub payload_json: String,
    pub attempt_count: i64,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct GmailModifyGroup {
    provider_account_id: i64,
    add_label_ids: Vec<String>,
    remove_label_ids: Vec<String>,
    change_ids: Vec<i64>,
    provider_message_ids: Vec<String>,
}

#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct ProviderBidiSyncSummary {
    pub selected: usize,
    pub applied: usize,
    pub failed: usize,
    pub cancelled: bool,
}

#[derive(Debug, Error)]
pub enum ProviderBidiSyncError {
    #[error("database error during provider bidi sync: {0}")]
    Database(#[from] sqlx::Error),
    #[error("gmail bidirectional sync push failed: {0}")]
    Gmail(#[from] GmailClientError),
}

#[async_trait]
pub trait GmailBidiSyncClient: Send + Sync {
    async fn batch_modify_messages(
        &self,
        request: &BatchModifyMessagesRequest,
    ) -> Result<(), GmailClientError>;
}

#[async_trait]
impl<T> GmailBidiSyncClient for GmailClient<T>
where
    T: GmailTokenSource,
{
    async fn batch_modify_messages(
        &self,
        request: &BatchModifyMessagesRequest,
    ) -> Result<(), GmailClientError> {
        self.batch_modify_messages(request).await
    }
}

pub async fn run_provider_bidi_sync_once(
    db: &SqlitePool,
    clients: &HashMap<i64, &(dyn GmailBidiSyncClient + Send + Sync)>,
    cancel: &CancellationToken,
) -> Result<ProviderBidiSyncSummary, ProviderBidiSyncError> {
    let changes = load_pending_changes(db, MAX_BATCH_IDS).await?;
    process_pending_changes(db, clients, changes, cancel).await
}

pub async fn process_pending_changes(
    db: &SqlitePool,
    clients: &HashMap<i64, &(dyn GmailBidiSyncClient + Send + Sync)>,
    changes: Vec<ProviderOutboundChange>,
    cancel: &CancellationToken,
) -> Result<ProviderBidiSyncSummary, ProviderBidiSyncError> {
    let mut summary = ProviderBidiSyncSummary {
        selected: changes.len(),
        ..ProviderBidiSyncSummary::default()
    };
    let groups = group_for_gmail_batch_modify(changes);

    for group in groups {
        if cancel.is_cancelled() {
            summary.cancelled = true;
            break;
        }
        let Some(client) = clients.get(&group.provider_account_id) else {
            mark_failed(
                db,
                &group.change_ids,
                "gmail client not available for provider account",
            )
            .await?;
            summary.failed += group.change_ids.len();
            continue;
        };
        let request = BatchModifyMessagesRequest {
            ids: group.provider_message_ids.clone(),
            add_label_ids: group.add_label_ids.clone(),
            remove_label_ids: group.remove_label_ids.clone(),
        };
        match cancel_or_complete(cancel, client.batch_modify_messages(&request)).await {
            None => {
                summary.cancelled = true;
                break;
            }
            Some(Ok(())) => {
                mark_applied(db, &group.change_ids).await?;
                summary.applied += group.change_ids.len();
            }
            Some(Err(error)) => {
                let message = safe_provider_account_error_message(&error);
                mark_failed(db, &group.change_ids, &message).await?;
                summary.failed += group.change_ids.len();
            }
        }
    }

    Ok(summary)
}

pub async fn run_provider_bidi_sync_loop<F, Fut>(
    db: SqlitePool,
    mut client_factory: F,
    cancel: CancellationToken,
) -> Result<(), ProviderBidiSyncError>
where
    F: FnMut(i64) -> Fut,
    Fut: std::future::Future<Output = Option<Box<dyn GmailBidiSyncClient + Send + Sync>>>,
{
    loop {
        if cancel.is_cancelled() {
            return Ok(());
        }
        let provider_account_ids = load_accounts_with_pending_changes(&db).await?;
        if provider_account_ids.is_empty() {
            cancel_aware_sleep(DEFAULT_IDLE_INTERVAL, &cancel).await;
            continue;
        }

        let mut owned_clients: Vec<(i64, Box<dyn GmailBidiSyncClient + Send + Sync>)> = Vec::new();
        for provider_account_id in provider_account_ids {
            if let Some(client) = client_factory(provider_account_id).await {
                owned_clients.push((provider_account_id, client));
            }
        }
        let client_refs = owned_clients
            .iter()
            .map(|(id, client)| (*id, client.as_ref()))
            .collect::<HashMap<_, _>>();
        let summary = run_provider_bidi_sync_once(&db, &client_refs, &cancel).await?;
        if summary.applied > 0 || summary.failed > 0 {
            info!(applied = summary.applied, failed = summary.failed, "provider bidi sync processed");
        }
        if summary.cancelled {
            return Ok(());
        }
    }
}

async fn load_accounts_with_pending_changes(db: &SqlitePool) -> Result<Vec<i64>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT DISTINCT provider_account_id FROM provider_outbound_changes WHERE applied_at IS NULL ORDER BY provider_account_id",
    )
    .fetch_all(db)
    .await
}

async fn load_pending_changes(
    db: &SqlitePool,
    limit: usize,
) -> Result<Vec<ProviderOutboundChange>, sqlx::Error> {
    let now = chrono::Utc::now().to_rfc3339();
    let rows = sqlx::query(
        "SELECT poc.id, poc.provider_account_id, poc.jmap_email_id, pmm.provider_message_id, \
                poc.change_type, poc.payload_json, poc.attempt_count \
         FROM provider_outbound_changes poc \
         INNER JOIN provider_message_mappings pmm \
           ON pmm.provider_account_id = poc.provider_account_id \
          AND pmm.jmap_email_id = poc.jmap_email_id \
         INNER JOIN mail_accounts pa ON pa.id = poc.provider_account_id \
         WHERE poc.applied_at IS NULL \
           AND pa.bidirectional_sync_enabled = 1 \
           AND (poc.attempt_count = 0 OR datetime(poc.created_at, '+' || MIN(?1, (1 << MIN(poc.attempt_count, 10))) || ' seconds') <= datetime(?2)) \
         ORDER BY poc.id \
         LIMIT ?3",
    )
    .bind(MAX_BACKOFF_SECS)
    .bind(now)
    .bind(i64::try_from(limit).unwrap_or(i64::MAX))
    .fetch_all(db)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| ProviderOutboundChange {
            id: row.get("id"),
            provider_account_id: row.get("provider_account_id"),
            jmap_email_id: row.get("jmap_email_id"),
            provider_message_id: row.get("provider_message_id"),
            change_type: row.get("change_type"),
            payload_json: row.get("payload_json"),
            attempt_count: row.get("attempt_count"),
        })
        .collect())
}

fn group_for_gmail_batch_modify(changes: Vec<ProviderOutboundChange>) -> Vec<GmailModifyGroup> {
    let mut groups: BTreeMap<(i64, Vec<String>, Vec<String>), GmailModifyGroup> = BTreeMap::new();
    for change in changes {
        let (add_label_ids, remove_label_ids) = gmail_label_delta(&change);
        let key = (
            change.provider_account_id,
            add_label_ids.clone(),
            remove_label_ids.clone(),
        );
        let group = groups.entry(key).or_insert_with(|| GmailModifyGroup {
            provider_account_id: change.provider_account_id,
            add_label_ids,
            remove_label_ids,
            change_ids: Vec::new(),
            provider_message_ids: Vec::new(),
        });
        group.change_ids.push(change.id);
        group.provider_message_ids.push(change.provider_message_id);
    }
    groups.into_values().collect()
}

fn gmail_label_delta(change: &ProviderOutboundChange) -> (Vec<String>, Vec<String>) {
    match change.change_type.as_str() {
        "read" => (Vec::new(), vec!["UNREAD".to_owned()]),
        "unread" => (vec!["UNREAD".to_owned()], Vec::new()),
        "trash" => (vec!["TRASH".to_owned()], Vec::new()),
        "untrash" => (Vec::new(), vec!["TRASH".to_owned()]),
        "label_add" => (payload_label_name(&change.payload_json).into_iter().collect(), Vec::new()),
        "label_remove" => (Vec::new(), payload_label_name(&change.payload_json).into_iter().collect()),
        _ => (Vec::new(), Vec::new()),
    }
}

#[derive(Deserialize)]
struct LabelPayload {
    label_name: Option<String>,
}

fn payload_label_name(payload_json: &str) -> Option<String> {
    serde_json::from_str::<LabelPayload>(payload_json)
        .ok()
        .and_then(|payload| payload.label_name)
        .filter(|name| !name.trim().is_empty())
}

async fn mark_applied(db: &SqlitePool, ids: &[i64]) -> Result<(), sqlx::Error> {
    if ids.is_empty() {
        return Ok(());
    }
    let now = chrono::Utc::now().to_rfc3339();
    let mut builder = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
        "UPDATE provider_outbound_changes SET applied_at = ",
    );
    builder.push_bind(now);
    builder.push(", last_error = NULL WHERE id IN (");
    let mut separated = builder.separated(", ");
    for id in ids {
        separated.push_bind(id);
    }
    separated.push_unseparated(")");
    builder.build().execute(db).await?;
    Ok(())
}

async fn mark_failed(db: &SqlitePool, ids: &[i64], error: &str) -> Result<(), sqlx::Error> {
    if ids.is_empty() {
        return Ok(());
    }
    let mut builder = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
        "UPDATE provider_outbound_changes SET attempt_count = attempt_count + 1, last_error = ",
    );
    builder.push_bind(error);
    builder.push(" WHERE id IN (");
    let mut separated = builder.separated(", ");
    for id in ids {
        separated.push_bind(id);
    }
    separated.push_unseparated(")");
    builder.build().execute(db).await?;
    Ok(())
}

async fn cancel_or_complete<T>(
    cancel: &CancellationToken,
    future: impl std::future::Future<Output = T>,
) -> Option<T> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => None,
        output = future => Some(output),
    }
}

async fn cancel_aware_sleep(delay: Duration, cancel: &CancellationToken) {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => {}
        _ = tokio::time::sleep(delay) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::StatusCode;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    #[allow(dead_code)]
    struct FakeGmail {
        calls: Arc<Mutex<Vec<BatchModifyMessagesRequest>>>,
        fail: bool,
    }

    #[async_trait]
    impl GmailBidiSyncClient for FakeGmail {
        async fn batch_modify_messages(
            &self,
            request: &BatchModifyMessagesRequest,
        ) -> Result<(), GmailClientError> {
            self.calls.lock().expect("calls").push(request.clone());
            if self.fail {
                Err(GmailClientError::Api {
                    status: StatusCode::BAD_REQUEST,
                    kind: crate::gmail_client::GmailApiErrorKind::BadRequest,
                    reason: Some("badRequest".to_owned()),
                    message: "invalid label".to_owned(),
                    retry_after: None,
                })
            } else {
                Ok(())
            }
        }
    }

    fn change(id: i64, change_type: &str, provider_message_id: &str) -> ProviderOutboundChange {
        ProviderOutboundChange {
            id,
            provider_account_id: 1,
            jmap_email_id: format!("email-{id}"),
            provider_message_id: provider_message_id.to_owned(),
            change_type: change_type.to_owned(),
            payload_json: if change_type.starts_with("label_") {
                r#"{"label_name":"Work"}"#.to_owned()
            } else {
                "{}".to_owned()
            },
            attempt_count: 0,
        }
    }

    #[test]
    fn groups_read_and_unread_into_idempotent_gmail_label_deltas() {
        let groups = group_for_gmail_batch_modify(vec![
            change(1, "read", "gm-1"),
            change(2, "read", "gm-2"),
            change(3, "unread", "gm-3"),
        ]);
        assert_eq!(groups.len(), 2);
        assert!(groups.iter().any(|group| group.remove_label_ids == ["UNREAD"] && group.provider_message_ids == ["gm-1", "gm-2"]));
        assert!(groups.iter().any(|group| group.add_label_ids == ["UNREAD"] && group.provider_message_ids == ["gm-3"]));
    }
}
