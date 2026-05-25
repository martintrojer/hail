//! Scheduled worker jobs for bubble-up reminders, scheduled sends, and trash purge.
//!
//! The scheduler owns hail-side state transitions for due rows:
//! - `bubble_ups`: query due pending rows, ask JMAP to resurface the
//!   corresponding thread to Imbox by making it unread and resetting hail
//!   classification keywords, then stamp `fired_at` and clear sidecar pile
//!   ordering state.
//! - `scheduled_sends`: atomically claim due `status='pending'` rows as
//!   `processing`, ask JMAP to submit the saved draft, then mark sent or failed.
//! - trash purge: ask JMAP to permanently destroy emails in the Trash mailbox
//!   whose `receivedAt` is older than the configured retention window.
//!
//! Failure policy is intentionally split by operation. Bubble-up JMAP failures
//! are treated as transient per design.md §8.3: the row remains pending and the
//! rest of the batch continues. Scheduled-send submission failures are classified
//! by the [`SendSubmitter`]: transient failures (network/server/rate/no active
//! session) are released back to pending for a later tick; permanent failures
//! (accepted JMAP request rejects the draft/recipients/identity, or the
//! scheduled send's accepting session is no longer available) become
//! `status='failed'` or `status='auth_required'` with `error` set. A crashed
//! worker can also leave a row in `processing` after taking the durable claim.
//! Because an EmailSubmission call
//! might already have reached JMAP, stale processing claims are not retried
//! automatically; after the claim timeout, or when the claim timestamp is
//! missing, the row is failed with an explicit unknown-submission-state error so
//! an operator/user can review it without risking duplicate mail. In all failure
//! cases, later due rows continue processing.

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde_json::json;
use sqlx::SqlitePool;
use tracing::{info, warn};

use crate::app_events::{WorkerAppEvent, publish_app_event, publish_app_event_payload};

pub const DEFAULT_TRASH_RETENTION_DAYS: u16 = 30;
const STALE_SCHEDULED_SEND_CLAIM_AFTER_SECS: i64 = 60 * 60;
const STALE_SCHEDULED_SEND_CLAIM_ERROR: &str = "scheduled send processing claim is stale or missing claimed_at; submission state unknown; manual review required";

#[derive(Debug, Clone, PartialEq, Eq)]
struct BubbleUpRow {
    id: i64,
    user_id: i64,
    thread_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ScheduledSendRow {
    id: i64,
    user_id: i64,
    draft_email_id: String,
}

#[async_trait]
pub trait BubbleJmapOps: Send + Sync {
    async fn resurface_thread(&self, user_id: i64, thread_id: &str) -> Result<()>;
}

#[async_trait]
pub trait TrashPurgeOps: Send + Sync {
    async fn purge_old_trash(&self, user_id: i64, cutoff: DateTime<Utc>) -> Result<usize>;
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SendSubmitError {
    #[error("transient submit failure: {0}")]
    Transient(String),
    #[error("scheduled send requires fresh authentication: {0}")]
    AuthRequired(String),
    #[error("permanent submit failure: {0}")]
    Permanent(String),
}

impl SendSubmitError {
    #[must_use]
    pub fn transient(error: impl std::fmt::Display) -> Self {
        Self::Transient(error.to_string())
    }

    #[must_use]
    pub fn auth_required(error: impl std::fmt::Display) -> Self {
        Self::AuthRequired(error.to_string())
    }

    #[must_use]
    pub fn permanent(error: impl std::fmt::Display) -> Self {
        Self::Permanent(error.to_string())
    }

    #[must_use]
    fn message(&self) -> &str {
        match self {
            Self::Transient(message) | Self::Permanent(message) | Self::AuthRequired(message) => {
                message
            }
        }
    }
}

#[async_trait]
pub trait SendSubmitter: Send + Sync {
    async fn submit_draft(
        &self,
        scheduled_send_id: i64,
        user_id: i64,
        draft_email_id: &str,
    ) -> std::result::Result<Option<String>, SendSubmitError>;
}

/// Process all pending scheduled sends whose `send_at` is due at or before `now`.
///
/// Each row is first atomically claimed with a durable `processing` state before
/// the non-idempotent JMAP submission call. Competing worker ticks/processes
/// that race on the same due row can observe it, but only one can transition it
/// from `pending` to `processing`; losers skip submission. Processing claims
/// older than one hour, or claims missing `claimed_at`, are recovered before due
/// pending rows are selected by failing the row with an explicit
/// unknown-submission-state error instead of retrying a potentially already
/// submitted draft. Transient submit failures release the row back to `pending`
/// for a later tick. Permanent failures mark the row `failed` and store the
/// error string. Later due rows are always attempted.
pub async fn process_due_scheduled_sends(
    db: &SqlitePool,
    submitter: &dyn SendSubmitter,
    now: DateTime<Utc>,
) -> Result<usize> {
    let now_s = now.to_rfc3339();
    recover_stale_scheduled_send_claims(db, now).await?;
    let rows = select_due_scheduled_sends(db, &now_s).await?;

    let mut sent = 0;
    for row in rows {
        if !claim_scheduled_send(db, row.id, &now_s).await? {
            continue;
        }

        match submitter
            .submit_draft(row.id, row.user_id, &row.draft_email_id)
            .await
        {
            Ok(submission_id) => {
                let result = sqlx::query(
                    "UPDATE scheduled_sends \
                     SET status = 'sent', sent_at = ?, error = NULL \
                     WHERE id = ? AND status = 'processing'",
                )
                .bind(&now_s)
                .bind(row.id)
                .execute(db)
                .await
                .with_context(|| format!("mark scheduled_send {} sent", row.id))?;
                if result.rows_affected() > 0 {
                    sent += 1;
                    record_scheduled_send_audit(
                        db,
                        row.user_id,
                        "compose.send_later.sent",
                        json!({
                            "scheduled_send_id": row.id,
                            "draft_email_id": row.draft_email_id,
                            "submission_id": submission_id,
                            "sent_at": now_s,
                        }),
                    )
                    .await;
                    if let Err(err) = publish_app_event_payload(
                        db,
                        row.user_id,
                        WorkerAppEvent::SendCompleted,
                        json!({
                            "scheduled_send_id": row.id,
                            "draft_email_id": row.draft_email_id,
                        }),
                    )
                    .await
                    {
                        warn!(
                            scheduled_send_id = row.id,
                            user_id = row.user_id,
                            error = %err,
                            "failed to publish scheduled send completed app event"
                        );
                    }
                    info!(
                        scheduled_send_id = row.id,
                        user_id = row.user_id,
                        draft_email_id = %row.draft_email_id,
                        submission_id = ?submission_id,
                        "scheduled send submitted"
                    );
                }
            }
            Err(SendSubmitError::Transient(message)) => {
                sqlx::query(
                    "UPDATE scheduled_sends \
                     SET status = 'pending', claimed_at = NULL, error = NULL \
                     WHERE id = ? AND status = 'processing'",
                )
                .bind(row.id)
                .execute(db)
                .await
                .with_context(|| {
                    format!("release scheduled_send {} after transient failure", row.id)
                })?;
                warn!(
                    scheduled_send_id = row.id,
                    user_id = row.user_id,
                    draft_email_id = %row.draft_email_id,
                    error = %message,
                    "scheduled send submit failed transiently; released for retry"
                );
            }
            Err(SendSubmitError::AuthRequired(message)) => {
                let result = sqlx::query(
                    "UPDATE scheduled_sends \
                     SET status = 'auth_required', error = ? \
                     WHERE id = ? AND status = 'processing'",
                )
                .bind(&message)
                .bind(row.id)
                .execute(db)
                .await
                .with_context(|| format!("mark scheduled_send {} auth_required", row.id))?;
                if result.rows_affected() > 0 {
                    if let Err(err) =
                        publish_app_event(db, row.user_id, WorkerAppEvent::SendFailed).await
                    {
                        warn!(
                            scheduled_send_id = row.id,
                            user_id = row.user_id,
                            error = %err,
                            "failed to publish scheduled send auth-required app event"
                        );
                    }
                    warn!(
                        scheduled_send_id = row.id,
                        user_id = row.user_id,
                        draft_email_id = %row.draft_email_id,
                        error = %message,
                        "scheduled send requires a fresh login before it can be retried"
                    );
                }
            }
            Err(err @ SendSubmitError::Permanent(_)) => {
                let message = err.message().to_string();
                let result = sqlx::query(
                    "UPDATE scheduled_sends \
                     SET status = 'failed', error = ? \
                     WHERE id = ? AND status = 'processing'",
                )
                .bind(&message)
                .bind(row.id)
                .execute(db)
                .await
                .with_context(|| format!("mark scheduled_send {} failed", row.id))?;
                if result.rows_affected() > 0 {
                    record_scheduled_send_audit(
                        db,
                        row.user_id,
                        "compose.send_later.failed",
                        json!({
                            "scheduled_send_id": row.id,
                            "draft_email_id": row.draft_email_id,
                            "error": message,
                        }),
                    )
                    .await;
                    if let Err(err) = publish_app_event_payload(
                        db,
                        row.user_id,
                        WorkerAppEvent::SendFailed,
                        json!({
                            "scheduled_send_id": row.id,
                            "draft_email_id": row.draft_email_id,
                            "error": message,
                        }),
                    )
                    .await
                    {
                        warn!(
                            scheduled_send_id = row.id,
                            user_id = row.user_id,
                            error = %err,
                            "failed to publish scheduled send failed app event"
                        );
                    }
                    warn!(
                        scheduled_send_id = row.id,
                        user_id = row.user_id,
                        draft_email_id = %row.draft_email_id,
                        error = %message,
                        "scheduled send submit failed permanently"
                    );
                }
            }
        }
    }

    Ok(sent)
}

async fn record_scheduled_send_audit(
    db: &SqlitePool,
    user_id: i64,
    action: &str,
    payload: serde_json::Value,
) {
    let payload_json = payload.to_string();
    let now = Utc::now().to_rfc3339();
    if let Err(err) = sqlx::query(
        "INSERT INTO audit_log (user_id, action, payload_json, created_at) \
         VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(user_id)
    .bind(action)
    .bind(payload_json)
    .bind(now)
    .execute(db)
    .await
    {
        warn!(user_id, action, error = %err, "audit log write failed");
    }
}

async fn recover_stale_scheduled_send_claims(db: &SqlitePool, now: DateTime<Utc>) -> Result<()> {
    let cutoff = (now - Duration::seconds(STALE_SCHEDULED_SEND_CLAIM_AFTER_SECS)).to_rfc3339();
    let stale_rows = sqlx::query_as::<_, (i64, i64, String)>(
        "SELECT id, user_id, draft_email_id \
         FROM scheduled_sends \
         WHERE status = 'processing' AND (claimed_at IS NULL OR claimed_at <= ?) \
         ORDER BY id ASC",
    )
    .bind(&cutoff)
    .fetch_all(db)
    .await
    .context("select stale scheduled_send processing claims")?;

    let mut recovered = 0_u64;
    for (id, user_id, draft_email_id) in stale_rows {
        let result = sqlx::query(
            "UPDATE scheduled_sends \
             SET status = 'failed', error = ? \
             WHERE id = ? AND status = 'processing' AND (claimed_at IS NULL OR claimed_at <= ?)",
        )
        .bind(STALE_SCHEDULED_SEND_CLAIM_ERROR)
        .bind(id)
        .bind(&cutoff)
        .execute(db)
        .await
        .with_context(|| format!("recover stale scheduled_send {id} processing claim"))?;

        if result.rows_affected() == 0 {
            continue;
        }
        recovered += 1;
        record_scheduled_send_audit(
            db,
            user_id,
            "compose.send_later.failed",
            json!({
                "scheduled_send_id": id,
                "draft_email_id": draft_email_id,
                "error": STALE_SCHEDULED_SEND_CLAIM_ERROR,
            }),
        )
        .await;
        if let Err(err) = publish_app_event_payload(
            db,
            user_id,
            WorkerAppEvent::SendFailed,
            json!({
                "scheduled_send_id": id,
                "draft_email_id": draft_email_id,
                "error": STALE_SCHEDULED_SEND_CLAIM_ERROR,
            }),
        )
        .await
        {
            warn!(
                scheduled_send_id = id,
                user_id,
                error = %err,
                "failed to publish stale scheduled send failed app event"
            );
        }
    }

    if recovered > 0 {
        warn!(
            recovered,
            stale_after_seconds = STALE_SCHEDULED_SEND_CLAIM_AFTER_SECS,
            "recovered stale scheduled send processing claims as failed with unknown submission state"
        );
    }

    Ok(())
}

async fn claim_scheduled_send(db: &SqlitePool, id: i64, now: &str) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE scheduled_sends \
         SET status = 'processing', claimed_at = ?, error = NULL \
         WHERE id = ? AND status = 'pending'",
    )
    .bind(now)
    .bind(id)
    .execute(db)
    .await
    .with_context(|| format!("claim scheduled_send {id}"))?;
    Ok(result.rows_affected() == 1)
}

async fn select_due_scheduled_sends(db: &SqlitePool, now: &str) -> Result<Vec<ScheduledSendRow>> {
    sqlx::query_as::<_, (i64, i64, String)>(
        "SELECT id, user_id, draft_email_id \
         FROM scheduled_sends \
         WHERE status = 'pending' AND send_at <= ? \
         ORDER BY send_at ASC, id ASC",
    )
    .bind(now)
    .fetch_all(db)
    .await
    .context("select due scheduled_sends")
    .map(|rows| {
        rows.into_iter()
            .map(|(id, user_id, draft_email_id)| ScheduledSendRow {
                id,
                user_id,
                draft_email_id,
            })
            .collect()
    })
}

/// Process all pending bubble-ups whose `surface_at` is due at or before `now`.
///
/// Returns the number of rows successfully fired. A transient JMAP failure for
/// one row leaves that row pending (`fired_at IS NULL`), logs a warning, and does
/// not stop later due rows from being processed. Successfully fired rows also
/// remove any sidecar pile ordering rows for that user/thread so the resurfaced
/// thread returns cleanly to Imbox.
pub async fn process_due_bubble_ups(
    db: &SqlitePool,
    jmap_ops: &dyn BubbleJmapOps,
    now: DateTime<Utc>,
) -> Result<usize> {
    let now_s = now.to_rfc3339();
    let rows = select_due_bubble_ups(db, &now_s).await?;

    let mut fired = 0;
    for row in rows {
        match jmap_ops.resurface_thread(row.user_id, &row.thread_id).await {
            Ok(()) => {
                let result = sqlx::query(
                    "UPDATE bubble_ups SET fired_at = ? WHERE id = ? AND fired_at IS NULL",
                )
                .bind(&now_s)
                .bind(row.id)
                .execute(db)
                .await
                .with_context(|| format!("mark bubble_up {} fired", row.id))?;
                if result.rows_affected() > 0 {
                    sqlx::query("DELETE FROM stack_positions WHERE user_id = ? AND thread_id = ?")
                        .bind(row.user_id)
                        .bind(&row.thread_id)
                        .execute(db)
                        .await
                        .with_context(|| {
                            format!(
                                "clear stack_positions for bubble_up {} user={} thread={}",
                                row.id, row.user_id, row.thread_id
                            )
                        })?;

                    fired += 1;
                    if let Err(err) =
                        publish_app_event(db, row.user_id, WorkerAppEvent::BubbleFired).await
                    {
                        warn!(
                            bubble_up_id = row.id,
                            user_id = row.user_id,
                            error = %err,
                            "failed to publish bubble-up fired app event"
                        );
                    }
                    info!(
                        bubble_up_id = row.id,
                        user_id = row.user_id,
                        thread_id = %row.thread_id,
                        "bubble-up fired"
                    );
                }
            }
            Err(err) => {
                warn!(
                    bubble_up_id = row.id,
                    user_id = row.user_id,
                    thread_id = %row.thread_id,
                    error = %err,
                    "bubble-up JMAP resurface failed; leaving pending"
                );
            }
        }
    }

    Ok(fired)
}

/// Permanently delete messages in JMAP Trash older than `retention_days`.
///
/// The JMAP adapter owns mailbox lookup, Email/query filtering, and Email/set
/// destroy. This scheduler function fans the purge out across hail users with an
/// active session. A per-user JMAP failure is logged and does not prevent later
/// users from being purged.
pub async fn process_trash_purge(
    db: &SqlitePool,
    jmap_ops: &dyn TrashPurgeOps,
    retention_days: u16,
    now: DateTime<Utc>,
) -> Result<usize> {
    let retention_days = i64::from(retention_days.max(1));
    let cutoff = now - Duration::days(retention_days);
    let user_ids = select_users_with_active_sessions(db, now).await?;

    let mut purged = 0;
    for user_id in user_ids {
        match jmap_ops.purge_old_trash(user_id, cutoff).await {
            Ok(count) => {
                purged += count;
                if count > 0 {
                    info!(user_id, count, "trash purge destroyed old messages");
                }
            }
            Err(err) => {
                warn!(
                    user_id,
                    error = %err,
                    "trash purge JMAP failure; continuing with later users"
                );
            }
        }
    }

    if purged > 0 {
        info!(purged, retention_days, "trash purge processed");
    }
    Ok(purged)
}

async fn select_users_with_active_sessions(
    db: &SqlitePool,
    now: DateTime<Utc>,
) -> Result<Vec<i64>> {
    sqlx::query_scalar::<_, i64>(
        "SELECT DISTINCT user_id FROM sessions WHERE expires_at > ? ORDER BY user_id ASC",
    )
    .bind(now.to_rfc3339())
    .fetch_all(db)
    .await
    .context("select users with active sessions")
}

async fn select_due_bubble_ups(db: &SqlitePool, now: &str) -> Result<Vec<BubbleUpRow>> {
    sqlx::query_as::<_, (i64, i64, String)>(
        "SELECT id, user_id, thread_id \
         FROM bubble_ups \
         WHERE fired_at IS NULL AND surface_at <= ? \
         ORDER BY surface_at ASC, id ASC",
    )
    .bind(now)
    .fetch_all(db)
    .await
    .context("select due bubble_ups")
    .map(|rows| {
        rows.into_iter()
            .map(|(id, user_id, thread_id)| BubbleUpRow {
                id,
                user_id,
                thread_id,
            })
            .collect()
    })
}

pub(crate) mod live {
    use std::sync::Arc;

    use anyhow::{Context, Result, anyhow};
    use async_trait::async_trait;
    use hail_jmap::jmap_client::Error as JmapClientError;
    use hail_jmap::jmap_client::core::query::Filter;
    use hail_jmap::jmap_client::core::set::{SetErrorType, SetObject};
    use hail_jmap::jmap_client::email::query as email_query;
    use hail_jmap::jmap_client::mailbox::Role;
    use secrecy::SecretString;
    use sqlx::SqlitePool;

    use super::{BubbleJmapOps, SendSubmitError, SendSubmitter, TrashPurgeOps};

    use crate::crypto::TokenDecryptor;

    /// Live JMAP adapter for bubble-ups.
    ///
    /// Implementation detail: JMAP has no direct "Thread/set" resurface verb.
    /// To return a thread to Imbox, query `Email/query` with Stalwart's
    /// `inThread` filter, then call `Email/set` for each returned Email to clear
    /// `$seen`, move it to the Inbox JMAP mailbox, set `$hail_imbox`, and remove
    /// other hail-owned classification or pile keywords. If Stalwart later exposes
    /// a first-class thread operation, this adapter is the only place that needs to
    /// change.
    pub struct LiveBubbleJmapOps {
        db: SqlitePool,
        jmap_url: String,
        token_decryptor: Arc<dyn TokenDecryptor>,
    }

    impl LiveBubbleJmapOps {
        #[must_use]
        pub fn new(
            db: SqlitePool,
            jmap_url: String,
            token_decryptor: Arc<dyn TokenDecryptor>,
        ) -> Self {
            Self {
                db,
                jmap_url,
                token_decryptor,
            }
        }

        async fn latest_active_token(&self, user_id: i64) -> Result<SecretString> {
            let now = chrono::Utc::now().to_rfc3339();
            let enc: Vec<u8> = sqlx::query_scalar(
                "SELECT jmap_token_enc FROM sessions \
                 WHERE user_id = ? AND expires_at > ? \
                 ORDER BY last_used_at DESC LIMIT 1",
            )
            .bind(user_id)
            .bind(now)
            .fetch_optional(&self.db)
            .await
            .with_context(|| format!("select active JMAP token for user {user_id}"))?
            .ok_or_else(|| anyhow!("no active JMAP session for user {user_id}"))?;

            self.token_decryptor
                .decrypt(&enc)
                .with_context(|| format!("decrypt JMAP token for user {user_id}"))
        }
    }

    #[async_trait]
    impl BubbleJmapOps for LiveBubbleJmapOps {
        async fn resurface_thread(&self, user_id: i64, thread_id: &str) -> Result<()> {
            let token = self.latest_active_token(user_id).await?;
            let session = hail_jmap::login_bearer(&self.jmap_url, token)
                .await
                .with_context(|| format!("JMAP login for user {user_id}"))?;

            let mut query = session
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
                .with_context(|| format!("Email/query inThread={thread_id}"))?;
            let email_ids = query.take_ids();

            let inbox_id = hail_jmap::mailbox_id_by_role(
                &session,
                hail_jmap::jmap_client::mailbox::Role::Inbox,
            )
            .await
            .context("Mailbox/get role=inbox")?
            .ok_or_else(|| anyhow!("inbox mailbox not found"))?;

            for email_id in email_ids {
                session
                    .client()
                    .email_set_keyword(&email_id, "$seen", false)
                    .await
                    .with_context(|| format!("Email/set clear $seen for {email_id}"))?;
                session
                    .client()
                    .email_set_mailboxes(&email_id, [inbox_id.clone()])
                    .await
                    .with_context(|| format!("Email/set move {email_id} to Inbox"))?;
                session
                    .client()
                    .email_set_keyword(&email_id, "$hail_imbox", true)
                    .await
                    .with_context(|| format!("Email/set add $hail_imbox for {email_id}"))?;
                for keyword in [
                    "$hail_setaside",
                    "$hail_replylater",
                    "$hail_feed",
                    "$hail_papertrail",
                ] {
                    session
                        .client()
                        .email_set_keyword(&email_id, keyword, false)
                        .await
                        .with_context(|| format!("Email/set remove {keyword} for {email_id}"))?;
                }
            }

            Ok(())
        }
    }

    #[async_trait]
    impl TrashPurgeOps for LiveBubbleJmapOps {
        async fn purge_old_trash(
            &self,
            user_id: i64,
            cutoff: chrono::DateTime<chrono::Utc>,
        ) -> Result<usize> {
            let token = self.latest_active_token(user_id).await?;
            let session = hail_jmap::login_bearer(&self.jmap_url, token)
                .await
                .with_context(|| format!("JMAP login for user {user_id}"))?;

            let Some(trash_mailbox_id) = hail_jmap::mailbox_id_by_role(&session, Role::Trash)
                .await
                .context("Mailbox/get Trash role lookup")?
            else {
                return Ok(0);
            };

            let filter = Filter::and([
                Filter::from(email_query::Filter::in_mailbox(trash_mailbox_id)),
                Filter::from(email_query::Filter::before(cutoff.timestamp())),
            ]);
            let mut query = session
                .client()
                .email_query(
                    Some(filter),
                    Some([email_query::Comparator::received_at().ascending()]),
                )
                .await
                .context("Email/query old Trash messages")?;
            let email_ids = query.take_ids();
            let count = email_ids.len();

            for email_id in email_ids {
                session
                    .client()
                    .email_destroy(&email_id)
                    .await
                    .with_context(|| format!("Email/set destroy {email_id}"))?;
            }

            Ok(count)
        }
    }

    pub struct LiveSendSubmitter {
        db: SqlitePool,
        jmap_url: String,
        token_decryptor: Arc<dyn TokenDecryptor>,
    }

    impl LiveSendSubmitter {
        #[must_use]
        pub fn new(
            db: SqlitePool,
            jmap_url: String,
            token_decryptor: Arc<dyn TokenDecryptor>,
        ) -> Self {
            Self {
                db,
                jmap_url,
                token_decryptor,
            }
        }

        #[cfg(test)]
        #[allow(dead_code)]
        pub(crate) async fn latest_active_token_and_email(
            &self,
            user_id: i64,
        ) -> std::result::Result<(SecretString, String), SendSubmitError> {
            let now = chrono::Utc::now().to_rfc3339();
            let row: (Vec<u8>, String) = sqlx::query_as(
                "SELECT s.jmap_token_enc, u.email FROM sessions s \
                 JOIN users u ON u.id = s.user_id \
                 WHERE s.user_id = ? AND s.expires_at > ? \
                 ORDER BY s.last_used_at DESC LIMIT 1",
            )
            .bind(user_id)
            .bind(&now)
            .fetch_optional(&self.db)
            .await
            .map_err(|err| SendSubmitError::transient(format!("select active JMAP token: {err}")))?
            .ok_or_else(|| {
                SendSubmitError::transient(format!("no active JMAP session for user {user_id}"))
            })?;

            let (enc, email) = row;
            let token = self
                .token_decryptor
                .decrypt(&enc)
                .map_err(|err| SendSubmitError::transient(format!("decrypt JMAP token: {err}")))?;
            Ok((token, email))
        }

        pub(crate) async fn scheduled_token_and_email(
            &self,
            scheduled_send_id: i64,
            user_id: i64,
            draft_email_id: &str,
        ) -> std::result::Result<(SecretString, String), SendSubmitError> {
            let now = chrono::Utc::now().to_rfc3339();
            let row: (Vec<u8>, String) = sqlx::query_as(
                "SELECT s.jmap_token_enc, u.email FROM scheduled_sends ss \
                 JOIN sessions s ON s.id = ss.auth_session_id \
                 JOIN users u ON u.id = ss.user_id \
                 WHERE ss.id = ? AND ss.user_id = ? AND ss.draft_email_id = ? \
                   AND ss.status = 'processing' \
                   AND ss.auth_session_expires_at > ? AND s.expires_at > ? \
                 LIMIT 1",
            )
            .bind(scheduled_send_id)
            .bind(user_id)
            .bind(draft_email_id)
            .bind(&now)
            .bind(&now)
            .fetch_optional(&self.db)
            .await
            .map_err(|err| {
                SendSubmitError::transient(format!("select scheduled JMAP token: {err}"))
            })?
            .ok_or_else(|| {
                SendSubmitError::auth_required(format!(
                    "auth_required: scheduled send needs a fresh login for user {user_id}"
                ))
            })?;

            let (enc, email) = row;
            let token = self.token_decryptor.decrypt(&enc).map_err(|err| {
                SendSubmitError::auth_required(format!(
                    "auth_required: decrypt scheduled JMAP token: {err}"
                ))
            })?;
            Ok((token, email))
        }
    }

    #[async_trait]
    impl SendSubmitter for LiveSendSubmitter {
        async fn submit_draft(
            &self,
            scheduled_send_id: i64,
            user_id: i64,
            draft_email_id: &str,
        ) -> std::result::Result<Option<String>, SendSubmitError> {
            let (token, email) = self
                .scheduled_token_and_email(scheduled_send_id, user_id, draft_email_id)
                .await?;
            let session = hail_jmap::login_bearer(&self.jmap_url, token)
                .await
                .map_err(|err| SendSubmitError::transient(format!("JMAP login: {err}")))?;
            let identity_id = identity_id_for(&session, &email)
                .await
                .map_err(classify_jmap_submit_error)?;

            let mut request = session.client().build();
            let create_id = request
                .set_email_submission()
                .create()
                .email_id(draft_email_id)
                .identity_id(identity_id)
                .create_id()
                .ok_or_else(|| {
                    SendSubmitError::permanent("EmailSubmission/set create id missing")
                })?;
            let mut response = request
                .send_set_email_submission()
                .await
                .map_err(classify_jmap_submit_error)?;
            let mut created = response
                .created(&create_id)
                .map_err(classify_jmap_submit_error)?;
            let submission_id = created.take_id();
            Ok((!submission_id.is_empty()).then_some(submission_id))
        }
    }

    async fn identity_id_for(
        session: &hail_jmap::Session,
        from: &str,
    ) -> std::result::Result<String, JmapClientError> {
        let mut request = session.client().build();
        request.get_identity().properties([
            hail_jmap::jmap_client::identity::Property::Id,
            hail_jmap::jmap_client::identity::Property::Email,
        ]);
        let mut response = request.send_get_identity().await?;
        let mut identities = response.take_list();
        if let Some(index) = identities.iter().position(|identity| {
            identity
                .email()
                .is_some_and(|email| email.eq_ignore_ascii_case(from))
        }) {
            return Ok(identities[index].take_id());
        }
        identities
            .first_mut()
            .map(|identity| identity.take_id())
            .filter(|id| !id.is_empty())
            .ok_or_else(|| JmapClientError::Internal("identity not found".to_string()))
    }

    fn is_transient_set_error_type(error: &SetErrorType) -> bool {
        matches!(error, SetErrorType::RateLimit | SetErrorType::OverQuota)
    }

    fn classify_jmap_submit_error(err: JmapClientError) -> SendSubmitError {
        match &err {
            JmapClientError::Set(set_err) => {
                if is_transient_set_error_type(set_err.error()) {
                    SendSubmitError::transient(err)
                } else {
                    SendSubmitError::permanent(err)
                }
            }
            JmapClientError::Transport(_)
            | JmapClientError::Server(_)
            | JmapClientError::Problem(_)
            | JmapClientError::Method(_) => SendSubmitError::transient(err),
            JmapClientError::Parse(_) | JmapClientError::Internal(_) => {
                SendSubmitError::permanent(err)
            }
            JmapClientError::WebSocket(_) => SendSubmitError::transient(err),
        }
    }
    #[cfg(test)]
    #[allow(
        dead_code,
        reason = "integration tests include this module by path; binary test targets do not call the helper"
    )]
    pub(crate) fn classify_jmap_set_error_type_for_test(error: &SetErrorType) -> SendSubmitError {
        if is_transient_set_error_type(error) {
            SendSubmitError::transient(error)
        } else {
            SendSubmitError::permanent(error)
        }
    }

    #[cfg(test)]
    #[allow(
        dead_code,
        reason = "integration tests include this module by path; binary test targets do not call the helper"
    )]
    pub(crate) fn classify_jmap_submit_error_for_test(err: JmapClientError) -> SendSubmitError {
        classify_jmap_submit_error(err)
    }
}
