use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use hail_backend::{BackendMsgId, Keyword, MailBackend};
use hail_cache::{CachedMail, MailTarget};
use hail_core::MailClassification;
use hail_db::labels::normalize_label_path;
use serde::Deserialize;
use sqlx::{Row, SqliteConnection};
use tracing::warn;

use crate::screener::{JmapOps, RouteError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowMessageContext {
    pub email_id: String,
    pub thread_id: String,
    pub from: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub subject: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowEvaluation {
    pub matched_rule_id: Option<i64>,
    pub classification: Option<MailClassification>,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct WorkflowCondition {
    field: WorkflowConditionField,
    op: WorkflowConditionOp,
    value: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum WorkflowConditionField {
    From,
    To,
    Cc,
    Subject,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum WorkflowConditionOp {
    Contains,
    Equals,
}

#[derive(Debug, Clone, Deserialize)]
struct WorkflowAction {
    classify_as: Option<MailClassification>,
    add_label: Option<String>,
    auto_reply: Option<String>,
}

struct WorkflowRule {
    id: i64,
    name: String,
    conditions: Vec<WorkflowCondition>,
    action: WorkflowAction,
}

pub async fn evaluate_workflows<O>(
    conn: &mut SqliteConnection,
    ops: &O,
    user_id: i64,
    ctx: &WorkflowMessageContext,
) -> Result<WorkflowEvaluation, RouteError>
where
    O: WorkflowOps + ?Sized,
{
    let rules = load_enabled_rules(conn, user_id).await?;
    for rule in rules {
        if !rule_matches(&rule, ctx) {
            continue;
        }
        if rule.action.classify_as.is_none() && rule.action.add_label.is_none() {
            if rule.action.auto_reply.is_some() {
                warn!(
                    user_id,
                    workflow_rule_id = rule.id,
                    workflow_rule_name = %rule.name,
                    email_id = %ctx.email_id,
                    "workflow auto_reply action matched but is not implemented; continuing to later rules"
                );
                continue;
            }
            continue;
        }

        let mut classification = None;
        if let Some(classify_as) = rule.action.classify_as {
            ops.apply_classification(&ctx.email_id, classify_as).await?;
            classification = Some(classify_as);
        }

        let mut label = None;
        if let Some(add_label) = rule.action.add_label.as_deref() {
            let assigned = assign_label_name_to_thread(conn, user_id, &ctx.thread_id, add_label)
                .await
                .map_err(RouteError::Db)?;
            label = Some(assigned);
        }

        if rule.action.auto_reply.is_some() {
            warn!(
                user_id,
                workflow_rule_id = rule.id,
                workflow_rule_name = %rule.name,
                email_id = %ctx.email_id,
                "workflow auto_reply action matched but is not implemented; supported actions were applied"
            );
        }

        return Ok(WorkflowEvaluation {
            matched_rule_id: Some(rule.id),
            classification,
            label,
        });
    }

    Ok(WorkflowEvaluation {
        matched_rule_id: None,
        classification: None,
        label: None,
    })
}

async fn load_enabled_rules(
    conn: &mut SqliteConnection,
    user_id: i64,
) -> Result<Vec<WorkflowRule>, RouteError> {
    let rows = sqlx::query(
        "SELECT id, name, conditions_json, action_json FROM workflow_rules WHERE user_id = ?1 AND enabled = 1 ORDER BY id ASC",
    )
    .bind(user_id)
    .fetch_all(&mut *conn)
    .await?;

    let mut rules = Vec::with_capacity(rows.len());
    for row in rows {
        let id: i64 = row.get("id");
        let name: String = row.get("name");
        let conditions_json: String = row.get("conditions_json");
        let action_json: String = row.get("action_json");
        let conditions = serde_json::from_str(&conditions_json).map_err(|err| {
            RouteError::InvalidDecision(format!("invalid workflow rule {id} conditions: {err}"))
        })?;
        let action = serde_json::from_str(&action_json).map_err(|err| {
            RouteError::InvalidDecision(format!("invalid workflow rule {id} action: {err}"))
        })?;
        rules.push(WorkflowRule {
            id,
            name,
            conditions,
            action,
        });
    }
    Ok(rules)
}

fn rule_matches(rule: &WorkflowRule, ctx: &WorkflowMessageContext) -> bool {
    !rule.conditions.is_empty()
        && rule
            .conditions
            .iter()
            .all(|condition| condition_matches(condition, ctx))
}

fn condition_matches(condition: &WorkflowCondition, ctx: &WorkflowMessageContext) -> bool {
    let needle = condition.value.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return false;
    }

    match condition.field {
        WorkflowConditionField::From => value_matches(&ctx.from, &needle, condition.op),
        WorkflowConditionField::Subject => value_matches(&ctx.subject, &needle, condition.op),
        WorkflowConditionField::To => values_match(&ctx.to, &needle, condition.op),
        WorkflowConditionField::Cc => values_match(&ctx.cc, &needle, condition.op),
    }
}

fn values_match(values: &[String], needle: &str, op: WorkflowConditionOp) -> bool {
    values.iter().any(|value| value_matches(value, needle, op))
}

fn value_matches(value: &str, needle: &str, op: WorkflowConditionOp) -> bool {
    let haystack = value.to_ascii_lowercase();
    match op {
        WorkflowConditionOp::Contains => haystack.contains(needle),
        WorkflowConditionOp::Equals => haystack == needle,
    }
}

#[async_trait]
pub trait WorkflowOps: Send + Sync {
    async fn apply_classification(
        &self,
        email_id: &str,
        classification: MailClassification,
    ) -> Result<(), RouteError>;
}

#[async_trait]
impl<T> WorkflowOps for T
where
    T: JmapOps + Send + Sync + ?Sized,
{
    async fn apply_classification(
        &self,
        email_id: &str,
        classification: MailClassification,
    ) -> Result<(), RouteError> {
        for candidate in MailClassification::ALL {
            self.remove_keyword(email_id, candidate.keyword()).await?;
        }
        self.apply_keyword(email_id, classification.keyword()).await
    }
}

#[allow(dead_code)]
pub struct CacheWorkflowOps {
    cache: Arc<CachedMail>,
}

#[allow(dead_code)]
impl CacheWorkflowOps {
    #[must_use]
    pub fn new(cache: Arc<CachedMail>) -> Self {
        Self { cache }
    }
}

#[async_trait]
impl WorkflowOps for CacheWorkflowOps {
    async fn apply_classification(
        &self,
        email_id: &str,
        classification: MailClassification,
    ) -> Result<(), RouteError> {
        let remove = MailClassification::ALL
            .iter()
            .map(|classification| Keyword::from_classification(*classification))
            .collect::<Vec<_>>();
        let add = [Keyword::from_classification(classification)];
        let id = BackendMsgId::new(email_id.to_owned());
        self.cache
            .mutate_keywords(MailTarget::Message(&id), &add, &remove)
            .await
            .map_err(|err| RouteError::Jmap(err.to_string()))
    }
}

#[allow(dead_code)]
pub struct BackendWorkflowOps<'a> {
    backend: &'a dyn MailBackend,
}

#[allow(dead_code)]
impl<'a> BackendWorkflowOps<'a> {
    #[must_use]
    pub fn new(backend: &'a dyn MailBackend) -> Self {
        Self { backend }
    }
}

#[async_trait]
impl WorkflowOps for BackendWorkflowOps<'_> {
    async fn apply_classification(
        &self,
        email_id: &str,
        classification: MailClassification,
    ) -> Result<(), RouteError> {
        let remove = MailClassification::ALL
            .iter()
            .map(|classification| Keyword::from_classification(*classification))
            .collect::<Vec<_>>();
        let add = [Keyword::from_classification(classification)];
        self.backend
            .set_keywords(&BackendMsgId::new(email_id.to_owned()), &add, &remove)
            .await
            .map_err(|err| RouteError::Jmap(err.to_string()))
    }
}

async fn assign_label_name_to_thread(
    conn: &mut SqliteConnection,
    user_id: i64,
    thread_id: &str,
    label_name: &str,
) -> Result<String, sqlx::Error> {
    let path =
        normalize_label_path(label_name).map_err(|err| sqlx::Error::Decode(Box::new(err)))?;
    let label_id = upsert_manual_label(conn, user_id, &path.name, &path.normalized_name).await?;
    sqlx::query(
        "INSERT INTO thread_labels (user_id, thread_id, label_id) VALUES (?1, ?2, ?3) ON CONFLICT(user_id, thread_id, label_id) DO NOTHING",
    )
    .bind(user_id)
    .bind(thread_id)
    .bind(label_id)
    .execute(&mut *conn)
    .await?;
    Ok(path.name)
}

async fn upsert_manual_label(
    conn: &mut SqliteConnection,
    user_id: i64,
    name: &str,
    normalized_name: &str,
) -> Result<i64, sqlx::Error> {
    if let Some(id) = sqlx::query_scalar::<_, i64>(
        "SELECT id FROM labels WHERE user_id = ?1 AND normalized_name = ?2",
    )
    .bind(user_id)
    .bind(normalized_name)
    .fetch_optional(&mut *conn)
    .await?
    {
        return Ok(id);
    }

    let now = Utc::now().to_rfc3339();
    sqlx::query_scalar::<_, i64>(
        "INSERT INTO labels (user_id, name, normalized_name, source, created_at, updated_at) VALUES (?1, ?2, ?3, 'manual', ?4, ?4) ON CONFLICT(user_id, normalized_name) DO UPDATE SET updated_at = labels.updated_at RETURNING id",
    )
    .bind(user_id)
    .bind(name)
    .bind(normalized_name)
    .bind(now)
    .fetch_one(&mut *conn)
    .await
}
