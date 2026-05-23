//! Scheduled worker jobs for bubble-up reminders and scheduled sends.
//!
//! The scheduler owns hail-side state transitions for due rows:
//! - `bubble_ups`: query due pending rows, ask JMAP to make the corresponding
//!   thread unread, then stamp `fired_at`.
//! - `scheduled_sends`: atomically claim due `status='pending'` rows as
//!   `processing`, ask JMAP to submit the saved draft, then mark sent or failed.
//!
//! Failure policy is intentionally split by operation. Bubble-up JMAP failures
//! are treated as transient per design.md §8.3: the row remains pending and the
//! rest of the batch continues. Scheduled-send submission failures are classified
//! by the [`SendSubmitter`]: transient failures (network/server/rate/no active
//! session) are released back to pending for a later tick; permanent failures
//! (accepted JMAP request rejects the draft/recipients/identity) become
//! `status='failed'` with `error` set. In both cases, later due rows continue
//! processing.

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use tracing::{info, warn};

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
    async fn mark_thread_unread(&self, user_id: i64, thread_id: &str) -> Result<()>;
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SendSubmitError {
    #[error("transient submit failure: {0}")]
    Transient(String),
    #[error("permanent submit failure: {0}")]
    Permanent(String),
}

impl SendSubmitError {
    #[must_use]
    pub fn transient(error: impl std::fmt::Display) -> Self {
        Self::Transient(error.to_string())
    }

    #[must_use]
    pub fn permanent(error: impl std::fmt::Display) -> Self {
        Self::Permanent(error.to_string())
    }

    #[must_use]
    fn message(&self) -> &str {
        match self {
            Self::Transient(message) | Self::Permanent(message) => message,
        }
    }
}

#[async_trait]
pub trait SendSubmitter: Send + Sync {
    async fn submit_draft(
        &self,
        user_id: i64,
        draft_email_id: &str,
    ) -> std::result::Result<Option<String>, SendSubmitError>;
}

/// Process all pending scheduled sends whose `send_at` is due at or before `now`.
///
/// Each row is first atomically claimed with a durable `processing` state before
/// the non-idempotent JMAP submission call. Competing worker ticks/processes
/// that race on the same due row can observe it, but only one can transition it
/// from `pending` to `processing`; losers skip submission. Transient submit
/// failures release the row back to `pending` for a later tick. Permanent
/// failures mark the row `failed` and store the error string. Later due rows are
/// always attempted.
pub async fn process_due_scheduled_sends(
    db: &SqlitePool,
    submitter: &dyn SendSubmitter,
    now: DateTime<Utc>,
) -> Result<usize> {
    let now_s = now.to_rfc3339();
    let rows = select_due_scheduled_sends(db, &now_s).await?;

    let mut sent = 0;
    for row in rows {
        if !claim_scheduled_send(db, row.id, &now_s).await? {
            continue;
        }

        match submitter
            .submit_draft(row.user_id, &row.draft_email_id)
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
/// not stop later due rows from being processed.
pub async fn process_due_bubble_ups(
    db: &SqlitePool,
    jmap_ops: &dyn BubbleJmapOps,
    now: DateTime<Utc>,
) -> Result<usize> {
    let now_s = now.to_rfc3339();
    let rows = select_due_bubble_ups(db, &now_s).await?;

    let mut fired = 0;
    for row in rows {
        match jmap_ops
            .mark_thread_unread(row.user_id, &row.thread_id)
            .await
        {
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
                    fired += 1;
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
                    "bubble-up JMAP mark unread failed; leaving pending"
                );
            }
        }
    }

    Ok(fired)
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
    use secrecy::SecretString;
    use sqlx::SqlitePool;

    use super::{BubbleJmapOps, SendSubmitError, SendSubmitter};
    use crate::crypto::TokenDecryptor;

    /// Live JMAP adapter for bubble-ups.
    ///
    /// Implementation detail: JMAP has no direct "Thread/set" unread verb. To
    /// mark a thread unread, query `Email/query` with Stalwart's `inThread`
    /// filter, then call `Email/set` for each returned Email with keyword
    /// `$seen = false` (`email_set_keyword(id, "$seen", false)`). If Stalwart
    /// later exposes a first-class thread unread operation, this adapter is the
    /// only place that needs to change.
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
        async fn mark_thread_unread(&self, user_id: i64, thread_id: &str) -> Result<()> {
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

            for email_id in email_ids {
                session
                    .client()
                    .email_set_keyword(&email_id, "$seen", false)
                    .await
                    .with_context(|| format!("Email/set clear $seen for {email_id}"))?;
            }

            Ok(())
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
            .bind(now)
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
    }

    #[async_trait]
    impl SendSubmitter for LiveSendSubmitter {
        async fn submit_draft(
            &self,
            user_id: i64,
            draft_email_id: &str,
        ) -> std::result::Result<Option<String>, SendSubmitError> {
            let (token, email) = self.latest_active_token_and_email(user_id).await?;
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
