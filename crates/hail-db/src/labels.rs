//! Label persistence helpers.
//!
//! Labels are local, thread-level tags. Nested-looking labels such as
//! `Work/Receipts` are stored as one flat full path; `/` only affects display
//! and normalization.

use sqlx::{Row, SqlitePool};

#[derive(Debug, thiserror::Error)]
pub enum LabelDbError {
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error("invalid label name: {0}")]
    InvalidName(String),
    #[error("invalid thread id: {0}")]
    InvalidThreadId(String),
    #[error("invalid provider label id: {0}")]
    InvalidProviderLabelId(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedLabelPath {
    pub name: String,
    pub normalized_name: String,
    pub path_segments: Vec<String>,
}

impl NormalizedLabelPath {
    pub fn leaf_name(&self) -> &str {
        self.path_segments
            .last()
            .map(String::as_str)
            .expect("validated label paths always have at least one segment")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelSource {
    Manual,
    Gmail,
}

impl LabelSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Gmail => "gmail",
        }
    }

    fn from_db(value: String) -> Self {
        match value.as_str() {
            "manual" => Self::Manual,
            "gmail" => Self::Gmail,
            _ => unreachable!("labels.source is DB-constrained"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Label {
    pub id: i64,
    pub user_id: i64,
    pub name: String,
    pub normalized_name: String,
    pub source: LabelSource,
    pub provider_kind: Option<String>,
    pub provider_label_id: Option<String>,
    pub color: Option<String>,
    pub thread_count: i64,
    pub created_at: String,
    pub updated_at: String,
}

impl Label {
    pub fn path_segments(&self) -> Vec<String> {
        self.name.split('/').map(ToOwned::to_owned).collect()
    }

    pub fn leaf_name(&self) -> &str {
        self.name.rsplit('/').next().unwrap_or(&self.name)
    }
}

/// Normalize and validate a flat label full path.
///
/// Display names trim whitespace around the whole name and around each path
/// segment. The normalized uniqueness key additionally compares
/// case-insensitively and collapses internal whitespace runs per segment.
pub fn normalize_label_path(input: &str) -> Result<NormalizedLabelPath, LabelDbError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(LabelDbError::InvalidName(
            "label name must not be empty".to_owned(),
        ));
    }

    let mut path_segments = Vec::new();
    let mut normalized_segments = Vec::new();
    for raw_segment in trimmed.split('/') {
        let segment = raw_segment.trim();
        if segment.is_empty() {
            return Err(LabelDbError::InvalidName(
                "label path segments must not be empty".to_owned(),
            ));
        }
        path_segments.push(segment.to_owned());
        normalized_segments.push(collapse_whitespace(segment).to_lowercase());
    }

    Ok(NormalizedLabelPath {
        name: path_segments.join("/"),
        normalized_name: normalized_segments.join("/"),
        path_segments,
    })
}

pub async fn list_labels(db: &SqlitePool, user_id: i64) -> Result<Vec<Label>, LabelDbError> {
    let rows = sqlx::query(LIST_LABELS_SQL)
        .bind(user_id)
        .fetch_all(db)
        .await?;
    Ok(rows.into_iter().map(label_from_row).collect())
}

pub async fn get_label(
    db: &SqlitePool,
    user_id: i64,
    label_id: i64,
) -> Result<Label, LabelDbError> {
    let row = sqlx::query(GET_LABEL_SQL)
        .bind(user_id)
        .bind(label_id)
        .fetch_one(db)
        .await?;
    Ok(label_from_row(row))
}

pub async fn find_label_by_name(
    db: &SqlitePool,
    user_id: i64,
    name: &str,
) -> Result<Option<Label>, LabelDbError> {
    let path = normalize_label_path(name)?;
    label_by_normalized_name(db, user_id, &path.normalized_name).await
}

pub async fn create_label(
    db: &SqlitePool,
    user_id: i64,
    name: &str,
    color: Option<&str>,
) -> Result<Label, LabelDbError> {
    let path = normalize_label_path(name)?;
    let label_id = insert_label(db, user_id, &path, LabelSource::Manual, None, None, color).await?;
    get_label(db, user_id, label_id).await
}

/// Return an existing label with this normalized name or create it. Useful for
/// inline assignment flows where typing a label name is an upsert.
pub async fn upsert_manual_label(
    db: &SqlitePool,
    user_id: i64,
    name: &str,
    color: Option<&str>,
) -> Result<Label, LabelDbError> {
    let path = normalize_label_path(name)?;
    if let Some(label) = label_by_normalized_name(db, user_id, &path.normalized_name).await? {
        return Ok(label);
    }
    create_label(db, user_id, &path.name, color).await
}

/// Upsert a Gmail user label according to the provider-import merge rules.
pub async fn upsert_gmail_label(
    db: &SqlitePool,
    user_id: i64,
    provider_label_id: &str,
    name: &str,
    color: Option<&str>,
) -> Result<Label, LabelDbError> {
    let provider_label_id = validate_provider_label_id(provider_label_id)?;
    let path = normalize_label_path(name)?;

    if let Some(existing) =
        label_by_provider_identity(db, user_id, "gmail", provider_label_id).await?
    {
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "UPDATE labels SET name = ?1, normalized_name = ?2, source = 'gmail', color = ?3, updated_at = ?4 WHERE user_id = ?5 AND id = ?6",
        )
        .bind(&path.name)
        .bind(&path.normalized_name)
        .bind(color)
        .bind(now)
        .bind(user_id)
        .bind(existing.id)
        .execute(db)
        .await?;
        return get_label(db, user_id, existing.id).await;
    }

    if let Some(existing) = label_by_normalized_name(db, user_id, &path.normalized_name).await? {
        if existing.provider_kind.is_none() && existing.provider_label_id.is_none() {
            let now = chrono::Utc::now().to_rfc3339();
            sqlx::query(
                "UPDATE labels SET provider_kind = 'gmail', provider_label_id = ?1, updated_at = ?2 WHERE user_id = ?3 AND id = ?4",
            )
            .bind(provider_label_id)
            .bind(now)
            .bind(user_id)
            .bind(existing.id)
            .execute(db)
            .await?;
            return get_label(db, user_id, existing.id).await;
        }
        return Ok(existing);
    }

    let label_id = insert_label(
        db,
        user_id,
        &path,
        LabelSource::Gmail,
        Some("gmail"),
        Some(provider_label_id),
        color,
    )
    .await?;
    get_label(db, user_id, label_id).await
}

pub async fn rename_label(
    db: &SqlitePool,
    user_id: i64,
    label_id: i64,
    name: &str,
) -> Result<Label, LabelDbError> {
    let path = normalize_label_path(name)?;
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE labels SET name = ?1, normalized_name = ?2, updated_at = ?3 WHERE user_id = ?4 AND id = ?5 RETURNING id",
    )
    .bind(path.name)
    .bind(path.normalized_name)
    .bind(now)
    .bind(user_id)
    .bind(label_id)
    .fetch_one(db)
    .await?;
    get_label(db, user_id, label_id).await
}

pub async fn delete_label(
    db: &SqlitePool,
    user_id: i64,
    label_id: i64,
) -> Result<bool, LabelDbError> {
    let result = sqlx::query("DELETE FROM labels WHERE user_id = ?1 AND id = ?2")
        .bind(user_id)
        .bind(label_id)
        .execute(db)
        .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn assign_label_to_thread(
    db: &SqlitePool,
    user_id: i64,
    thread_id: &str,
    label_id: i64,
) -> Result<bool, LabelDbError> {
    let thread_id = validate_thread_id(thread_id)?;
    let result = sqlx::query(
        "INSERT INTO thread_labels (user_id, thread_id, label_id) SELECT ?1, ?2, l.id FROM labels l WHERE l.user_id = ?1 AND l.id = ?3 ON CONFLICT(user_id, thread_id, label_id) DO NOTHING",
    )
    .bind(user_id)
    .bind(thread_id)
    .bind(label_id)
    .execute(db)
    .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn remove_label_from_thread(
    db: &SqlitePool,
    user_id: i64,
    thread_id: &str,
    label_id: i64,
) -> Result<bool, LabelDbError> {
    let thread_id = validate_thread_id(thread_id)?;
    let result = sqlx::query(
        "DELETE FROM thread_labels WHERE user_id = ?1 AND thread_id = ?2 AND label_id = ?3",
    )
    .bind(user_id)
    .bind(thread_id)
    .bind(label_id)
    .execute(db)
    .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn list_thread_labels(
    db: &SqlitePool,
    user_id: i64,
    thread_id: &str,
) -> Result<Vec<Label>, LabelDbError> {
    let thread_id = validate_thread_id(thread_id)?;
    let rows = sqlx::query(LIST_THREAD_LABELS_SQL)
        .bind(user_id)
        .bind(thread_id)
        .fetch_all(db)
        .await?;
    Ok(rows.into_iter().map(label_from_row).collect())
}

pub async fn list_label_thread_ids(
    db: &SqlitePool,
    user_id: i64,
    label_id: i64,
    limit: i64,
    offset: i64,
) -> Result<Vec<String>, LabelDbError> {
    let rows = sqlx::query_scalar::<_, String>(
        "SELECT tl.thread_id FROM thread_labels tl INNER JOIN labels l ON l.user_id = tl.user_id AND l.id = tl.label_id WHERE tl.user_id = ?1 AND tl.label_id = ?2 ORDER BY tl.created_at DESC, tl.thread_id DESC LIMIT ?3 OFFSET ?4",
    )
    .bind(user_id)
    .bind(label_id)
    .bind(limit.max(0))
    .bind(offset.max(0))
    .fetch_all(db)
    .await?;
    Ok(rows)
}

pub async fn assign_label_to_threads(
    db: &SqlitePool,
    user_id: i64,
    thread_ids: &[&str],
    label_id: i64,
) -> Result<u64, LabelDbError> {
    let mut assigned = 0;
    for thread_id in thread_ids {
        if assign_label_to_thread(db, user_id, thread_id, label_id).await? {
            assigned += 1;
        }
    }
    Ok(assigned)
}

pub async fn assign_label_name_to_thread(
    db: &SqlitePool,
    user_id: i64,
    thread_id: &str,
    label_name: &str,
) -> Result<Label, LabelDbError> {
    let label = upsert_manual_label(db, user_id, label_name, None).await?;
    assign_label_to_thread(db, user_id, thread_id, label.id).await?;
    get_label(db, user_id, label.id).await
}

pub async fn assign_label_name_to_threads(
    db: &SqlitePool,
    user_id: i64,
    thread_ids: &[&str],
    label_name: &str,
) -> Result<Label, LabelDbError> {
    let label = upsert_manual_label(db, user_id, label_name, None).await?;
    assign_label_to_threads(db, user_id, thread_ids, label.id).await?;
    get_label(db, user_id, label.id).await
}

async fn insert_label(
    db: &SqlitePool,
    user_id: i64,
    path: &NormalizedLabelPath,
    source: LabelSource,
    provider_kind: Option<&str>,
    provider_label_id: Option<&str>,
    color: Option<&str>,
) -> Result<i64, LabelDbError> {
    let now = chrono::Utc::now().to_rfc3339();
    let label_id = sqlx::query_scalar(
        "INSERT INTO labels (user_id, name, normalized_name, source, provider_kind, provider_label_id, color, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8) RETURNING id",
    )
    .bind(user_id)
    .bind(&path.name)
    .bind(&path.normalized_name)
    .bind(source.as_str())
    .bind(provider_kind)
    .bind(provider_label_id)
    .bind(color)
    .bind(now)
    .fetch_one(db)
    .await?;
    Ok(label_id)
}

async fn label_by_normalized_name(
    db: &SqlitePool,
    user_id: i64,
    normalized_name: &str,
) -> Result<Option<Label>, LabelDbError> {
    let row = sqlx::query(LABEL_BY_NORMALIZED_NAME_SQL)
        .bind(user_id)
        .bind(normalized_name)
        .fetch_optional(db)
        .await?;
    Ok(row.map(label_from_row))
}

async fn label_by_provider_identity(
    db: &SqlitePool,
    user_id: i64,
    provider_kind: &str,
    provider_label_id: &str,
) -> Result<Option<Label>, LabelDbError> {
    let row = sqlx::query(LABEL_BY_PROVIDER_IDENTITY_SQL)
        .bind(user_id)
        .bind(provider_kind)
        .bind(provider_label_id)
        .fetch_optional(db)
        .await?;
    Ok(row.map(label_from_row))
}

fn label_from_row(row: sqlx::sqlite::SqliteRow) -> Label {
    Label {
        id: row.get("id"),
        user_id: row.get("user_id"),
        name: row.get("name"),
        normalized_name: row.get("normalized_name"),
        source: LabelSource::from_db(row.get("source")),
        provider_kind: row.get("provider_kind"),
        provider_label_id: row.get("provider_label_id"),
        color: row.get("color"),
        thread_count: row.get("thread_count"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn validate_thread_id(thread_id: &str) -> Result<&str, LabelDbError> {
    let trimmed = thread_id.trim();
    if trimmed.is_empty() {
        return Err(LabelDbError::InvalidThreadId(
            "thread id must not be empty".to_owned(),
        ));
    }
    if trimmed != thread_id {
        return Err(LabelDbError::InvalidThreadId(
            "thread id must not have leading or trailing whitespace".to_owned(),
        ));
    }
    Ok(trimmed)
}

fn validate_provider_label_id(provider_label_id: &str) -> Result<&str, LabelDbError> {
    let trimmed = provider_label_id.trim();
    if trimmed.is_empty() {
        return Err(LabelDbError::InvalidProviderLabelId(
            "provider label id must not be empty".to_owned(),
        ));
    }
    if trimmed != provider_label_id {
        return Err(LabelDbError::InvalidProviderLabelId(
            "provider label id must not have leading or trailing whitespace".to_owned(),
        ));
    }
    Ok(trimmed)
}

const LIST_LABELS_SQL: &str = concat!(
    "SELECT l.id, l.user_id, l.name, l.normalized_name, l.source, l.provider_kind, ",
    "l.provider_label_id, l.color, l.created_at, l.updated_at, ",
    "COUNT(tl.thread_id) AS thread_count FROM labels l ",
    "LEFT JOIN thread_labels tl ON tl.user_id = l.user_id AND tl.label_id = l.id ",
    "WHERE l.user_id = ?1 GROUP BY l.id ORDER BY l.normalized_name ASC"
);
const GET_LABEL_SQL: &str = concat!(
    "SELECT l.id, l.user_id, l.name, l.normalized_name, l.source, l.provider_kind, ",
    "l.provider_label_id, l.color, l.created_at, l.updated_at, ",
    "COUNT(tl.thread_id) AS thread_count FROM labels l ",
    "LEFT JOIN thread_labels tl ON tl.user_id = l.user_id AND tl.label_id = l.id ",
    "WHERE l.user_id = ?1 AND l.id = ?2 GROUP BY l.id"
);
const LABEL_BY_NORMALIZED_NAME_SQL: &str = concat!(
    "SELECT l.id, l.user_id, l.name, l.normalized_name, l.source, l.provider_kind, ",
    "l.provider_label_id, l.color, l.created_at, l.updated_at, ",
    "COUNT(tl.thread_id) AS thread_count FROM labels l ",
    "LEFT JOIN thread_labels tl ON tl.user_id = l.user_id AND tl.label_id = l.id ",
    "WHERE l.user_id = ?1 AND l.normalized_name = ?2 GROUP BY l.id"
);
const LABEL_BY_PROVIDER_IDENTITY_SQL: &str = concat!(
    "SELECT l.id, l.user_id, l.name, l.normalized_name, l.source, l.provider_kind, ",
    "l.provider_label_id, l.color, l.created_at, l.updated_at, ",
    "COUNT(tl.thread_id) AS thread_count FROM labels l ",
    "LEFT JOIN thread_labels tl ON tl.user_id = l.user_id AND tl.label_id = l.id ",
    "WHERE l.user_id = ?1 AND l.provider_kind = ?2 AND l.provider_label_id = ?3 GROUP BY l.id"
);
const LIST_THREAD_LABELS_SQL: &str = concat!(
    "SELECT l.id, l.user_id, l.name, l.normalized_name, l.source, l.provider_kind, ",
    "l.provider_label_id, l.color, l.created_at, l.updated_at, ",
    "COUNT(tl.thread_id) AS thread_count FROM labels l ",
    "LEFT JOIN thread_labels tl ON tl.user_id = l.user_id AND tl.label_id = l.id ",
    "INNER JOIN thread_labels tl_filter ON tl_filter.user_id = l.user_id AND tl_filter.label_id = l.id ",
    "WHERE l.user_id = ?1 AND tl_filter.thread_id = ?2 GROUP BY l.id ORDER BY l.normalized_name ASC"
);
