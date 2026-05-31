use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::middleware::auth::AuthUser;
use crate::routes::response::{bad_request, internal};
use crate::state::AppState;

pub const TAG: &str = "users";

#[derive(Debug, Clone, Serialize, ToSchema, PartialEq, Eq)]
pub struct UserPrefsResponse {
    pub feed_load_remote_images: bool,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateUserPrefsRequest {
    pub feed_load_remote_images: Option<bool>,
}

pub fn router() -> Router<AppState> {
    Router::from(openapi_router())
}

pub fn openapi_router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(get_user_prefs, patch_user_prefs))
}

#[utoipa::path(
    get,
    path = "/api/user/prefs",
    tag = TAG,
    responses(
        (status = 200, description = "Current user's UI preferences.", body = UserPrefsResponse),
        (status = 401, description = "Missing or invalid session."),
        (status = 500, description = "Preference lookup failed."),
    ),
)]
async fn get_user_prefs(State(state): State<AppState>, axum::Extension(user): axum::Extension<AuthUser>) -> Response {
    match load_user_prefs(&state.db, user.id).await {
        Ok(prefs) => Json(prefs).into_response(),
        Err(err) => {
            tracing::error!(user_id = user.id, error = %err, "user prefs lookup failed");
            internal()
        }
    }
}

#[utoipa::path(
    patch,
    path = "/api/user/prefs",
    tag = TAG,
    request_body = UpdateUserPrefsRequest,
    responses(
        (status = 200, description = "Updated current user's UI preferences.", body = UserPrefsResponse),
        (status = 400, description = "Invalid preference payload."),
        (status = 401, description = "Missing or invalid session."),
        (status = 403, description = "Missing CSRF header."),
        (status = 500, description = "Preference update failed."),
    ),
)]
async fn patch_user_prefs(
    State(state): State<AppState>,
    axum::Extension(user): axum::Extension<AuthUser>,
    Json(payload): Json<UpdateUserPrefsRequest>,
) -> Response {
    let Some(feed_load_remote_images) = payload.feed_load_remote_images else {
        return bad_request("empty_prefs_patch");
    };

    match save_user_prefs(&state.db, user.id, feed_load_remote_images).await {
        Ok(prefs) => Json(prefs).into_response(),
        Err(err) => {
            tracing::error!(user_id = user.id, error = %err, "user prefs update failed");
            internal()
        }
    }
}

pub async fn load_user_prefs(
    db: &sqlx::SqlitePool,
    user_id: i64,
) -> Result<UserPrefsResponse, sqlx::Error> {
    let feed_load_remote_images = sqlx::query_scalar::<_, bool>(
        "SELECT COALESCE(feed_load_remote_images, 0) FROM user_prefs WHERE user_id = ?1",
    )
    .bind(user_id)
    .fetch_optional(db)
    .await?
    .unwrap_or(false);

    Ok(UserPrefsResponse {
        feed_load_remote_images,
    })
}

pub async fn save_user_prefs(
    db: &sqlx::SqlitePool,
    user_id: i64,
    feed_load_remote_images: bool,
) -> Result<UserPrefsResponse, sqlx::Error> {
    sqlx::query(
        "INSERT INTO user_prefs (user_id, feed_load_remote_images) VALUES (?1, ?2) \
         ON CONFLICT(user_id) DO UPDATE SET feed_load_remote_images = excluded.feed_load_remote_images",
    )
    .bind(user_id)
    .bind(feed_load_remote_images)
    .execute(db)
    .await?;

    Ok(UserPrefsResponse {
        feed_load_remote_images,
    })
}
