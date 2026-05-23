use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Classification {
    Imbox,
    Feed,
    Papertrail,
}

impl Classification {
    #[must_use]
    pub fn keyword(&self) -> &'static str {
        match self {
            Self::Imbox => "$hail_imbox",
            Self::Feed => "$hail_feed",
            Self::Papertrail => "$hail_papertrail",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "imbox" => Some(Self::Imbox),
            "feed" => Some(Self::Feed),
            "papertrail" => Some(Self::Papertrail),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteOutcome {
    Classified { classification: Classification },
    Trashed,
    ScreenerPending { sender: String },
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

#[must_use]
pub fn normalize_sender(addr: &str) -> String {
    let trimmed = addr.trim();
    let email = match (trimmed.rfind('<'), trimmed.rfind('>')) {
        (Some(start), Some(end)) if start < end => &trimmed[start + 1..end],
        _ => trimmed,
    };
    email.trim().to_ascii_lowercase()
}

pub async fn route_email(
    conn: &mut SqliteConnection,
    jmap: &dyn JmapOps,
    user_id: i64,
    env: &EmailEnvelope,
) -> Result<RouteOutcome, RouteError> {
    if env.keywords.iter().any(|kw| kw.starts_with("$hail_")) {
        return Ok(RouteOutcome::AlreadyScreened);
    }

    let row: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT decision, classify_as FROM screener_rules WHERE user_id = ? AND sender_address = ?",
    )
    .bind(user_id)
    .bind(&env.from)
    .fetch_optional(&mut *conn)
    .await?;

    match row {
        Some((decision, classify_as)) if decision == "allow" => {
            let classify_as = classify_as.ok_or_else(|| {
                RouteError::InvalidClassification("allow rule missing classify_as".to_string())
            })?;
            let classification = Classification::parse(&classify_as)
                .ok_or(RouteError::InvalidClassification(classify_as))?;
            jmap.apply_keyword(&env.id, classification.keyword())
                .await?;
            Ok(RouteOutcome::Classified { classification })
        }
        Some((decision, _)) if decision == "deny" => {
            let trash_id = jmap
                .get_mailbox_by_role("trash")
                .await?
                .ok_or_else(|| RouteError::MissingMailbox("trash".to_string()))?;
            jmap.move_to_mailbox(&env.id, &trash_id).await?;
            Ok(RouteOutcome::Trashed)
        }
        Some((decision, _)) if decision == "pending" => {
            move_to_screener_if_needed(jmap, env).await?;
            Ok(RouteOutcome::ScreenerPending {
                sender: env.from.clone(),
            })
        }
        Some((decision, _)) => Err(RouteError::InvalidDecision(decision)),
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
            .bind(&env.from)
            .bind(first_seen_at)
            .execute(&mut *conn)
            .await?;
            Ok(RouteOutcome::ScreenerPending {
                sender: env.from.clone(),
            })
        }
    }
}

async fn move_to_screener_if_needed(
    jmap: &dyn JmapOps,
    env: &EmailEnvelope,
) -> Result<(), RouteError> {
    let screener_id = jmap.get_or_create_mailbox("Screener").await?;
    if env
        .mailbox_ids
        .iter()
        .any(|mailbox_id| mailbox_id == &screener_id)
    {
        return Ok(());
    }
    jmap.move_to_mailbox(&env.id, &screener_id).await
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
