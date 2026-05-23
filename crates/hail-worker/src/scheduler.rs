//! Scheduled worker jobs for bubble-up reminders.
//!
//! The scheduler owns the hail-side state transition for due `bubble_ups` rows:
//! query due pending rows, ask JMAP to make the corresponding thread unread, then
//! stamp `fired_at`. JMAP failures are treated as transient per design.md §8.3:
//! the row remains pending and the rest of the batch continues.

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

#[async_trait]
pub trait BubbleJmapOps: Send + Sync {
    async fn mark_thread_unread(&self, user_id: i64, thread_id: &str) -> Result<()>;
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
        match jmap_ops.mark_thread_unread(row.user_id, &row.thread_id).await {
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

mod live {
    use std::sync::Arc;

    use anyhow::{Context, Result, anyhow};
    use async_trait::async_trait;
    use hail_jmap::jmap_client::core::query::Filter;
    use hail_jmap::jmap_client::email::query as email_query;
    use secrecy::SecretString;
    use sqlx::SqlitePool;

    use super::BubbleJmapOps;
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
                    None::<Vec<hail_jmap::jmap_client::core::query::Comparator<
                        email_query::Comparator,
                    >>>,
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
}

    #[allow(unused_imports)]
    pub use live::LiveBubbleJmapOps;
