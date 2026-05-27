//! Speakeasy current passphrase API.
//!
//! Speakeasy is a monthly rotating Screener bypass passphrase. Returning the
//! raw current phrase to the authenticated user is intentional: the UI must be
//! able to show the secret so the user can share it out-of-band. We therefore
//! avoid writing the phrase to logs and keep mutation audit payloads secret-free.

use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use rand::TryRngCore;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::middleware::auth::AuthUser;
use crate::routes::response::{bad_request, internal};
use crate::state::AppState;

/// OpenAPI tag for Speakeasy bypass-passphrase endpoints.
pub const TAG: &str = "speakeasy";

const WORDS: &[&str] = &[
    "amber", "atlas", "basil", "beacon", "birch", "breeze", "canyon", "cedar", "cinder", "clover",
    "cobalt", "comet", "copper", "coral", "cricket", "daisy", "delta", "ember", "falcon", "fennel",
    "fjord", "forest", "ginger", "harbor", "hazel", "indigo", "juniper", "lagoon", "laurel",
    "linen", "maple", "marble", "meadow", "meteor", "nectar", "olive", "onyx", "orchid", "pepper",
    "prairie", "quartz", "raven", "river", "saffron", "silver", "spruce", "summit", "thistle",
    "violet", "willow", "zephyr",
];

/// Build protected Speakeasy routes.
pub fn router() -> Router<AppState> {
    Router::from(openapi_router())
}

/// Build the OpenAPI-tracked router for protected Speakeasy routes.
pub fn openapi_router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(get_speakeasy, rotate_speakeasy))
}

#[derive(Debug, Serialize, ToSchema)]
struct SpeakeasyResponse {
    speakeasy: SpeakeasyState,
}

#[derive(Debug, Serialize, ToSchema)]
struct SpeakeasyState {
    /// Raw current bypass passphrase. This is intentionally returned only to
    /// the authenticated owner so the UI can display/share it.
    passphrase: String,
    /// UTC month this phrase is current for, formatted YYYY-MM.
    period: String,
    #[schema(value_type = String, format = DateTime)]
    rotates_at: DateTime<Utc>,
    #[schema(value_type = String, format = DateTime)]
    generated_at: DateTime<Utc>,
    #[schema(value_type = Option<String>, format = DateTime)]
    manually_rotated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize, ToSchema)]
struct RotateSpeakeasyRequest {
    /// Optional explicit acknowledgement that this invalidates the previous
    /// phrase immediately. Omitted/false is still accepted for initial API
    /// clients; the field exists so the SPA can make the warning explicit.
    #[serde(default)]
    acknowledge_bypass_secret: bool,
}

#[utoipa::path(
    get,
    path = "/api/speakeasy",
    tag = TAG,
    responses(
        (status = 200, description = "Current Speakeasy bypass passphrase and rotation metadata.", body = SpeakeasyResponse),
        (status = 401, description = "Missing or invalid session."),
        (status = 500, description = "Speakeasy lookup failed."),
    ),
)]
async fn get_speakeasy(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Response {
    let now = Utc::now();
    let row = match hail_db::speakeasy::current_or_create_speakeasy_passphrase(
        &state.db,
        user.id,
        now,
        generate_passphrase,
    )
    .await
    {
        Ok(row) => row,
        Err(err) => {
            tracing::error!(user_id = user.id, error = %err, "speakeasy lookup failed");
            return internal();
        }
    };

    Json(SpeakeasyResponse {
        speakeasy: row.into(),
    })
    .into_response()
}

#[utoipa::path(
    post,
    path = "/api/speakeasy/rotate",
    tag = TAG,
    request_body(content = RotateSpeakeasyRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "Speakeasy passphrase rotated immediately.", body = SpeakeasyResponse),
        (status = 400, description = "Invalid JSON payload."),
        (status = 401, description = "Missing or invalid session."),
        (status = 403, description = "CSRF header missing."),
        (status = 500, description = "Speakeasy rotation failed."),
    ),
)]
async fn rotate_speakeasy(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    body: Result<Option<Json<RotateSpeakeasyRequest>>, JsonRejection>,
) -> Response {
    let _acknowledged = match body {
        Ok(Some(Json(body))) => body.acknowledge_bypass_secret,
        Ok(None) => false,
        Err(_) => return bad_request("invalid_json"),
    };

    let now = Utc::now();
    let row = match hail_db::speakeasy::rotate_speakeasy_passphrase(
        &state.db,
        user.id,
        now,
        generate_passphrase,
    )
    .await
    {
        Ok(row) => row,
        Err(err) => {
            tracing::error!(user_id = user.id, error = %err, "speakeasy rotation failed");
            return internal();
        }
    };

    if let Err(err) = crate::audit::record(
        &state.db,
        user.id,
        "speakeasy.rotate",
        &serde_json::json!({ "period": row.period, "rotates_at": row.rotates_at }),
    )
    .await
    {
        tracing::warn!(user_id = user.id, error = %err, "failed to audit speakeasy rotation");
    }

    (
        StatusCode::OK,
        Json(SpeakeasyResponse {
            speakeasy: row.into(),
        }),
    )
        .into_response()
}

impl From<hail_db::speakeasy::SpeakeasyPassphrase> for SpeakeasyState {
    fn from(value: hail_db::speakeasy::SpeakeasyPassphrase) -> Self {
        Self {
            passphrase: value.passphrase,
            period: value.period,
            rotates_at: value.rotates_at,
            generated_at: value.generated_at,
            manually_rotated_at: value.manually_rotated_at,
        }
    }
}

fn generate_passphrase() -> String {
    let mut indexes = [0u8; 4];
    let mut suffix = [0u8; 8];
    rand::rngs::OsRng
        .try_fill_bytes(&mut indexes)
        .expect("OS RNG available for Speakeasy passphrase generation");
    rand::rngs::OsRng
        .try_fill_bytes(&mut suffix)
        .expect("OS RNG available for Speakeasy passphrase generation");
    let mut parts = indexes
        .into_iter()
        .map(|byte| WORDS[usize::from(byte) % WORDS.len()].to_owned())
        .collect::<Vec<_>>();
    parts.push(hex::encode(suffix));
    parts.join("-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_passphrase_is_human_readable_and_long_enough() {
        let phrase = generate_passphrase();
        assert!(phrase.len() >= 33);
        assert_eq!(phrase.split('-').count(), 5);
    }
}
