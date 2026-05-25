use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use secrecy::SecretString;
use sqlx::SqlitePool;

use crate::crypto::TokenDecryptor;

pub async fn latest_active_token(
    db: &SqlitePool,
    token_decryptor: &Arc<dyn TokenDecryptor>,
    user_id: i64,
) -> Result<SecretString> {
    let now = chrono::Utc::now().to_rfc3339();
    let enc: Vec<u8> = sqlx::query_scalar(
        "SELECT jmap_token_enc FROM sessions \
         WHERE user_id = ? AND expires_at > ? \
         ORDER BY last_used_at DESC LIMIT 1",
    )
    .bind(user_id)
    .bind(now)
    .fetch_optional(db)
    .await
    .with_context(|| format!("select active JMAP token for user {user_id}"))?
    .ok_or_else(|| anyhow!("no active JMAP session for user {user_id}"))?;

    token_decryptor
        .decrypt(&enc)
        .with_context(|| format!("decrypt JMAP token for user {user_id}"))
}
