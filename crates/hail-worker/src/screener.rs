use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
pub use hail_core::screener::{Classification, normalize_sender};
use hail_core::screener::{ScreenerDecision, ScreenerRule, ScreenerRuleParseError, lookup_rule};
use hail_core::{HAIL_SPAM_KEYWORD, SPAM_KEYWORD};
use hail_jmap::jmap_client;
use hail_jmap::jmap_client::mailbox::{Role, query::Filter};
use sqlx::SqliteConnection;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmailEnvelope {
    pub id: String,
    pub thread_id: String,
    pub from: String,
    pub subject: String,
    pub mailbox_ids: Vec<String>,
    pub keywords: Vec<String>,
    pub received_at: Option<DateTime<Utc>>,
    pub size: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteOutcome {
    Classified { classification: Classification },
    Trashed,
    ScreenerPending { sender: String },
    Spam,
    AlreadyScreened,
}

#[derive(Debug, thiserror::Error)]
pub enum RouteError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("jmap error: {0}")]
    Jmap(String),
    #[error("required mailbox missing: {0}")]
    MissingMailbox(String),
    #[error("invalid classification: {0}")]
    InvalidClassification(String),
    #[error("invalid screener decision: {0}")]
    InvalidDecision(String),
}

impl From<jmap_client::Error> for RouteError {
    fn from(value: jmap_client::Error) -> Self {
        Self::Jmap(value.to_string())
    }
}

#[async_trait]
pub trait JmapOps: Send + Sync {
    async fn get_or_create_mailbox(&self, name: &str) -> Result<String, RouteError>;
    async fn get_mailbox_by_role(&self, role: &str) -> Result<Option<String>, RouteError>;
    async fn apply_keyword(&self, email_id: &str, keyword: &str) -> Result<(), RouteError>;
    async fn remove_keyword(&self, email_id: &str, keyword: &str) -> Result<(), RouteError>;
    async fn move_to_mailbox(&self, email_id: &str, mailbox_id: &str) -> Result<(), RouteError>;
}

pub struct JmapOpsLive {
    pub session: Arc<hail_jmap::Session>,
    pub account_id: String,
}

#[async_trait]
impl JmapOps for JmapOpsLive {
    async fn get_or_create_mailbox(&self, name: &str) -> Result<String, RouteError> {
        let mut query = self
            .session
            .client()
            .mailbox_query(Some(Filter::name(name)), None::<Vec<_>>)
            .await?;
        if let Some(id) = query.take_ids().into_iter().next() {
            return Ok(id);
        }

        let mailbox = self
            .session
            .client()
            .mailbox_create(name, None::<String>, Role::None)
            .await?;
        mailbox.id().map(str::to_string).ok_or_else(|| {
            RouteError::Jmap("mailbox_create returned mailbox without id".to_string())
        })
    }

    async fn get_mailbox_by_role(&self, role: &str) -> Result<Option<String>, RouteError> {
        let role = parse_mailbox_role(role)?;
        let mut query = self
            .session
            .client()
            .mailbox_query(Some(Filter::role(role)), None::<Vec<_>>)
            .await?;
        Ok(query.take_ids().into_iter().next())
    }

    async fn apply_keyword(&self, email_id: &str, keyword: &str) -> Result<(), RouteError> {
        self.session
            .client()
            .email_set_keyword(email_id, keyword, true)
            .await?;
        Ok(())
    }

    async fn remove_keyword(&self, email_id: &str, keyword: &str) -> Result<(), RouteError> {
        self.session
            .client()
            .email_set_keyword(email_id, keyword, false)
            .await?;
        Ok(())
    }

    async fn move_to_mailbox(&self, email_id: &str, mailbox_id: &str) -> Result<(), RouteError> {
        self.session
            .client()
            .email_set_mailboxes(email_id, [mailbox_id.to_string()])
            .await?;
        Ok(())
    }
}

impl JmapOpsLive {
    #[allow(dead_code)]
    #[must_use]
    pub fn account_id(&self) -> &str {
        &self.account_id
    }
}

/// System senders whose mail should bypass the screener and go straight to Imbox.
/// Covers bounce notifications (mailer-daemon), postmaster, and null-sender bounces.
fn is_system_sender(from: &str) -> bool {
    let lower = from.to_ascii_lowercase();
    let local = lower.split('@').next().unwrap_or(&lower);
    matches!(
        local,
        "mailer-daemon" | "postmaster" | "noreply" | "no-reply"
    ) || lower.is_empty()
}

#[must_use]
pub fn is_spam_flagged(keywords: &[String]) -> bool {
    keywords
        .iter()
        .any(|keyword| keyword == SPAM_KEYWORD || keyword == "Junk")
}

pub async fn route_email(
    conn: &mut SqliteConnection,
    jmap: &dyn JmapOps,
    user_id: i64,
    env: &EmailEnvelope,
) -> Result<RouteOutcome, RouteError> {
    if env.keywords.iter().any(|kw| kw == HAIL_SPAM_KEYWORD) {
        return Ok(RouteOutcome::AlreadyScreened);
    }

    if is_spam_flagged(&env.keywords) {
        let junk_id = match jmap.get_mailbox_by_role("junk").await? {
            Some(id) => id,
            None => jmap.get_or_create_mailbox("Junk").await?,
        };
        jmap.move_to_mailbox(&env.id, &junk_id).await?;
        jmap.apply_keyword(&env.id, HAIL_SPAM_KEYWORD).await?;
        return Ok(RouteOutcome::Spam);
    }

    if env.keywords.iter().any(|kw| kw.starts_with("$hail_")) {
        return Ok(RouteOutcome::AlreadyScreened);
    }

    // System senders (bounce notifications, postmaster) bypass the screener entirely.
    if is_system_sender(&env.from) {
        jmap.apply_keyword(&env.id, Classification::Imbox.keyword())
            .await?;
        return Ok(RouteOutcome::Classified {
            classification: Classification::Imbox,
        });
    }

    let sender = normalize_sender(&env.from);
    let row: Option<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT sender_address, decision, classify_as FROM screener_rules WHERE user_id = ? AND sender_address = ?",
    )
    .bind(user_id)
    .bind(&sender)
    .fetch_optional(&mut *conn)
    .await?;

    let rules = match row {
        Some((sender_address, decision, classify_as)) => vec![
            ScreenerRule::from_db(sender_address, &decision, classify_as.as_deref()).map_err(
                |err| match err {
                    ScreenerRuleParseError::MissingClassification => {
                        RouteError::InvalidClassification(
                            "allow rule missing classify_as".to_string(),
                        )
                    }
                    ScreenerRuleParseError::InvalidClassification(value) => {
                        RouteError::InvalidClassification(value)
                    }
                    ScreenerRuleParseError::InvalidDecision(value) => {
                        RouteError::InvalidDecision(value)
                    }
                },
            )?,
        ],
        None => Vec::new(),
    };

    match lookup_rule(&rules, &sender) {
        Some(ScreenerDecision::Allow(classification)) => {
            jmap.apply_keyword(&env.id, classification.keyword())
                .await?;
            Ok(RouteOutcome::Classified { classification })
        }
        Some(ScreenerDecision::Deny) => {
            let trash_id = jmap
                .get_mailbox_by_role("trash")
                .await?
                .ok_or_else(|| RouteError::MissingMailbox("trash".to_string()))?;
            jmap.move_to_mailbox(&env.id, &trash_id).await?;
            Ok(RouteOutcome::Trashed)
        }
        Some(ScreenerDecision::Pending) => {
            move_to_screener_if_needed(jmap, env).await?;
            Ok(RouteOutcome::ScreenerPending {
                sender: sender.clone(),
            })
        }
        None => {
            move_to_screener_if_needed(jmap, env).await?;
            let first_seen_at = env.received_at.unwrap_or_else(Utc::now).to_rfc3339();
            sqlx::query(
                "INSERT INTO screener_rules \
                 (user_id, sender_address, decision, classify_as, decided_at, first_seen_at) \
                 VALUES (?, ?, 'pending', NULL, NULL, ?) \
                 ON CONFLICT(user_id, sender_address) DO NOTHING",
            )
            .bind(user_id)
            .bind(&sender)
            .bind(first_seen_at)
            .execute(&mut *conn)
            .await?;
            Ok(RouteOutcome::ScreenerPending { sender })
        }
    }
}

async fn move_to_screener_if_needed(
    jmap: &dyn JmapOps,
    env: &EmailEnvelope,
) -> Result<(), RouteError> {
    let screener_id = jmap
        .get_or_create_mailbox(hail_jmap::SCREENER_MAILBOX_NAME)
        .await?;
    if env
        .mailbox_ids
        .iter()
        .any(|mailbox_id| mailbox_id == &screener_id)
    {
        return Ok(());
    }
    jmap.move_to_mailbox(&env.id, &screener_id).await?;
    for classification in Classification::ALL {
        jmap.remove_keyword(&env.id, classification.keyword())
            .await?;
    }
    Ok(())
}

fn parse_mailbox_role(role: &str) -> Result<Role, RouteError> {
    match role.to_ascii_lowercase().as_str() {
        "archive" => Ok(Role::Archive),
        "drafts" => Ok(Role::Drafts),
        "important" => Ok(Role::Important),
        "inbox" => Ok(Role::Inbox),
        "junk" => Ok(Role::Junk),
        "sent" => Ok(Role::Sent),
        "trash" => Ok(Role::Trash),
        other => Err(RouteError::MissingMailbox(other.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_senders_detected() {
        assert!(is_system_sender("mailer-daemon@localhost.local"));
        assert!(is_system_sender("MAILER-DAEMON@example.com"));
        assert!(is_system_sender("Mailer-Daemon@mx.example.com"));
        assert!(is_system_sender("postmaster@example.com"));
        assert!(is_system_sender("POSTMASTER@example.com"));
        assert!(is_system_sender("noreply@example.com"));
        assert!(is_system_sender("no-reply@example.com"));
        assert!(is_system_sender("")); // null sender bounce
    }

    #[test]
    fn normal_senders_not_system() {
        assert!(!is_system_sender("alice@example.com"));
        assert!(!is_system_sender("newsletter@company.com"));
        assert!(!is_system_sender("daemon@example.com"));
        assert!(!is_system_sender("reply@example.com"));
    }
}
