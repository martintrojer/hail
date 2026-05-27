//! Speakeasy passphrase persistence helpers.
//!
//! Speakeasy is a per-user monthly Screener bypass secret. It is deliberately
//! modeled as one current passphrase per user, not sender routing state.

use chrono::{DateTime, Datelike, TimeZone, Utc};
use sqlx::SqlitePool;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpeakeasyPassphrase {
    pub user_id: i64,
    pub passphrase: String,
    pub period: String,
    pub rotates_at: DateTime<Utc>,
    pub generated_at: DateTime<Utc>,
    pub manually_rotated_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

pub async fn current_or_create_speakeasy_passphrase(
    db: &SqlitePool,
    user_id: i64,
    now: DateTime<Utc>,
    generate: impl FnOnce() -> String,
) -> Result<SpeakeasyPassphrase, sqlx::Error> {
    let period = period_for(now);
    if let Some(existing) = get_speakeasy_passphrase(db, user_id).await?
        && existing.period == period
        && existing.rotates_at > now
    {
        return Ok(existing);
    }

    upsert_speakeasy_passphrase(db, user_id, generate(), now, None).await
}

pub async fn rotate_speakeasy_passphrase(
    db: &SqlitePool,
    user_id: i64,
    now: DateTime<Utc>,
    generate: impl FnOnce() -> String,
) -> Result<SpeakeasyPassphrase, sqlx::Error> {
    upsert_speakeasy_passphrase(db, user_id, generate(), now, Some(now)).await
}

pub async fn get_speakeasy_passphrase(
    db: &SqlitePool,
    user_id: i64,
) -> Result<Option<SpeakeasyPassphrase>, sqlx::Error> {
    sqlx::query_as::<_, SpeakeasyPassphraseRow>(
        "SELECT user_id, passphrase, period, rotates_at, generated_at, manually_rotated_at, updated_at \
         FROM speakeasy_passphrases WHERE user_id = ?1",
    )
    .bind(user_id)
    .fetch_optional(db)
    .await
    .map(|row| row.map(Into::into))
}

async fn upsert_speakeasy_passphrase(
    db: &SqlitePool,
    user_id: i64,
    passphrase: String,
    now: DateTime<Utc>,
    manually_rotated_at: Option<DateTime<Utc>>,
) -> Result<SpeakeasyPassphrase, sqlx::Error> {
    let period = period_for(now);
    let rotates_at = next_period_start(now);
    sqlx::query_as::<_, SpeakeasyPassphraseRow>(
        "INSERT INTO speakeasy_passphrases \
         (user_id, passphrase, period, rotates_at, generated_at, manually_rotated_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?5) \
         ON CONFLICT(user_id) DO UPDATE SET \
           passphrase = excluded.passphrase, \
           period = excluded.period, \
           rotates_at = excluded.rotates_at, \
           generated_at = excluded.generated_at, \
           manually_rotated_at = excluded.manually_rotated_at, \
           updated_at = excluded.updated_at \
         RETURNING user_id, passphrase, period, rotates_at, generated_at, manually_rotated_at, updated_at",
    )
    .bind(user_id)
    .bind(passphrase)
    .bind(period)
    .bind(rotates_at)
    .bind(now)
    .bind(manually_rotated_at)
    .fetch_one(db)
    .await
    .map(Into::into)
}

fn period_for(now: DateTime<Utc>) -> String {
    format!("{:04}-{:02}", now.year(), now.month())
}

fn next_period_start(now: DateTime<Utc>) -> DateTime<Utc> {
    let (year, month) = if now.month() == 12 {
        (now.year() + 1, 1)
    } else {
        (now.year(), now.month() + 1)
    };
    Utc.with_ymd_and_hms(year, month, 1, 0, 0, 0)
        .single()
        .expect("valid first day of month")
}

#[derive(sqlx::FromRow)]
struct SpeakeasyPassphraseRow {
    user_id: i64,
    passphrase: String,
    period: String,
    rotates_at: DateTime<Utc>,
    generated_at: DateTime<Utc>,
    manually_rotated_at: Option<DateTime<Utc>>,
    updated_at: DateTime<Utc>,
}

impl From<SpeakeasyPassphraseRow> for SpeakeasyPassphrase {
    fn from(row: SpeakeasyPassphraseRow) -> Self {
        Self {
            user_id: row.user_id,
            passphrase: row.passphrase,
            period: row.period,
            rotates_at: row.rotates_at,
            generated_at: row.generated_at,
            manually_rotated_at: row.manually_rotated_at,
            updated_at: row.updated_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monthly_period_and_rotation_boundary_are_utc_month_based() {
        let now = Utc.with_ymd_and_hms(2026, 5, 27, 12, 30, 0).unwrap();
        assert_eq!(period_for(now), "2026-05");
        assert_eq!(
            next_period_start(now),
            Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap()
        );

        let december = Utc.with_ymd_and_hms(2026, 12, 31, 23, 59, 59).unwrap();
        assert_eq!(period_for(december), "2026-12");
        assert_eq!(
            next_period_start(december),
            Utc.with_ymd_and_hms(2027, 1, 1, 0, 0, 0).unwrap()
        );
    }
}
