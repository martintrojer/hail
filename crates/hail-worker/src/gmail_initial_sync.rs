//! Worker-facing orchestration for Gmail provider-account initial sync.
//!
//! The historical importer owns page-by-page Gmail `messages.list`, raw RFC822
//! fetches, Stalwart import, durable provider mappings, and backfill cursors.
//! This module adds the provider-account level wrapper used by worker jobs:
//! verify the Gmail profile for the connected account, persist the profile
//! high-water `historyId`, then run a bounded historical import. Raw RFC822 bytes
//! are passed only through the importer boundary and are never stored in hail.db.

use async_trait::async_trait;
use chrono::Utc;
use hail_db::provider_audit_sanitizer::safe_provider_account_error_message;
use sqlx::SqlitePool;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use crate::gmail_client::{GmailClient, GmailClientError, GmailProfile, GmailTokenSource};
use crate::gmail_historical_import::{
    GmailHistoricalImportAccount, GmailHistoricalImportError, GmailHistoricalImportOptions,
    GmailHistoricalImportSummary, GmailHistoricalImporter, GmailHistoricalSource,
    import_gmail_history,
};

const PROFILE_MISMATCH_ERROR_CLASS: &str = "gmail_profile_mismatch";
const PROFILE_ERROR_CLASS: &str = "gmail_profile";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GmailProviderAccount {
    pub id: i64,
    pub user_id: i64,
    pub provider_account_id: String,
    pub provider_email: String,
    pub jmap_account_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GmailInitialSyncOptions {
    pub historical: GmailHistoricalImportOptions,
}

impl GmailInitialSyncOptions {
    /// Build default inbound Gmail initial-sync options for the target local mailboxes.
    ///
    /// v1.2 provider import intentionally imports only Gmail's system `INBOX` label by
    /// default. Archived All Mail, Trash/Spam, and Sent are left out unless a future
    /// explicit import mode opts into a wider provider window.
    #[must_use]
    pub fn into_mailboxes(target_mailbox_ids: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let mut historical = GmailHistoricalImportOptions::into_mailboxes(target_mailbox_ids);
        historical.label_ids =
            vec![crate::gmail_historical_import::GMAIL_INBOX_LABEL_ID.to_owned()];
        Self { historical }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GmailInitialSyncSummary {
    pub profile: GmailProfile,
    pub import: GmailHistoricalImportSummary,
}

#[async_trait]
pub trait GmailInitialSyncSource: GmailHistoricalSource {
    async fn profile(&self) -> Result<GmailProfile, GmailClientError>;
}

#[async_trait]
impl<T> GmailInitialSyncSource for GmailClient<T>
where
    T: GmailTokenSource,
{
    async fn profile(&self) -> Result<GmailProfile, GmailClientError> {
        self.profile().await
    }
}

#[derive(Debug, Error)]
pub enum GmailInitialSyncError {
    #[error("gmail profile lookup failed: {0}")]
    Profile(#[source] GmailClientError),
    #[error(
        "gmail profile email {profile_email:?} does not match mail account {provider_email:?}"
    )]
    ProfileMismatch {
        profile_email: String,
        provider_email: String,
    },
    #[error(transparent)]
    Import(#[from] GmailHistoricalImportError),
    #[error("database error during Gmail initial sync: {0}")]
    Database(#[from] sqlx::Error),
}

pub async fn run_gmail_initial_sync<C, I>(
    db: &SqlitePool,
    account: GmailProviderAccount,
    gmail: &C,
    importer: &I,
    options: GmailInitialSyncOptions,
    cancel: &CancellationToken,
) -> Result<GmailInitialSyncSummary, GmailInitialSyncError>
where
    C: GmailInitialSyncSource,
    I: GmailHistoricalImporter,
{
    let profile = match gmail.profile().await {
        Ok(profile) => profile,
        Err(error) => {
            mark_initial_sync_error(
                db,
                account.id,
                PROFILE_ERROR_CLASS,
                &safe_error_message(&error),
            )
            .await?;
            return Err(GmailInitialSyncError::Profile(error));
        }
    };

    let profile_email = normalize_email(&profile.email_address);
    let provider_email = normalize_email(&account.provider_email);
    if profile_email != provider_email {
        let message = format!(
            "gmail profile email does not match connected mail account: {profile_email} != {provider_email}"
        );
        mark_initial_sync_error(db, account.id, PROFILE_MISMATCH_ERROR_CLASS, &message).await?;
        return Err(GmailInitialSyncError::ProfileMismatch {
            profile_email,
            provider_email,
        });
    }

    persist_profile_sync(db, account.id, profile.history_id.as_deref()).await?;

    let import = import_gmail_history(
        db,
        GmailHistoricalImportAccount {
            provider_account_id: account.id,
            user_id: account.user_id,
        },
        gmail,
        importer,
        options.historical,
        cancel,
    )
    .await?;

    Ok(GmailInitialSyncSummary { profile, import })
}

#[allow(dead_code)]
pub async fn load_gmail_provider_account(
    db: &SqlitePool,
    provider_account_row_id: i64,
) -> Result<Option<GmailProviderAccount>, sqlx::Error> {
    load_gmail_provider_account_by_id(db, provider_account_row_id).await
}

pub(crate) async fn load_gmail_provider_account_by_id(
    db: &SqlitePool,
    provider_account_row_id: i64,
) -> Result<Option<GmailProviderAccount>, sqlx::Error> {
    sqlx::query_as::<_, (i64, i64, String, String, String)>(
        "SELECT id, user_id, provider_account_id, provider_email, jmap_account_id \
         FROM mail_accounts \
         WHERE id = ?1 AND backend_kind = 'gmail' AND sync_status != 'disconnected'",
    )
    .bind(provider_account_row_id)
    .fetch_optional(db)
    .await
    .map(|row| {
        row.map(
            |(id, user_id, provider_account_id, provider_email, jmap_account_id)| {
                GmailProviderAccount {
                    id,
                    user_id,
                    provider_account_id,
                    provider_email,
                    jmap_account_id,
                }
            },
        )
    })
}

async fn persist_profile_sync(
    db: &SqlitePool,
    provider_account_id: i64,
    history_id: Option<&str>,
) -> Result<(), sqlx::Error> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE mail_accounts \
         SET last_profile_history_id = ?1, profile_synced_at = ?2, updated_at = ?2 \
         WHERE id = ?3",
    )
    .bind(history_id)
    .bind(now)
    .bind(provider_account_id)
    .execute(db)
    .await?;
    Ok(())
}

async fn mark_initial_sync_error(
    db: &SqlitePool,
    provider_account_id: i64,
    class: &str,
    message: &str,
) -> Result<(), sqlx::Error> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE mail_accounts \
         SET sync_status = 'error', last_error_class = ?1, last_error_message = ?2, updated_at = ?3 \
         WHERE id = ?4",
    )
    .bind(class)
    .bind(message)
    .bind(now)
    .bind(provider_account_id)
    .execute(db)
    .await?;
    Ok(())
}

fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

fn safe_error_message(error: &impl std::fmt::Display) -> String {
    safe_provider_account_error_message(error)
}
