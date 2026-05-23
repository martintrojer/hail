//! Thread verb endpoints.
//!
//! This module starts the `/api/threads/*` verb surface. Today it only
//! contains Bubble Up, but keeping thread verbs together avoids scattering
//! JMAP visibility checks as the API grows.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use axum::extract::{Extension, Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing::post};
use chrono::{DateTime, Duration, Utc};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::middleware::auth::AuthUser;
use crate::state::AppState;

/// Dependency-injection seam for `Thread/get`. Production uses
/// [`JmapThreadVerifier`]; tests can attach a fake verifier as a request
/// extension so no live Stalwart is needed.
pub trait ThreadVerifier: Send + Sync + 'static {
    fn exists<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        thread_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<bool, ThreadVerifyError>> + Send + 'a>>;
}

/// Production verifier: open a bearer JMAP session for the authenticated
/// user and issue `Thread/get(ids=[thread_id], properties=[id])`.
pub struct JmapThreadVerifier;

impl ThreadVerifier for JmapThreadVerifier {
    fn exists<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        thread_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<bool, ThreadVerifyError>> + Send + 'a>> {
        Box::pin(async move {
            let session = hail_jmap::login_bearer(&state.config.stalwart.jmap_url, token)
                .await
                .map_err(|err| ThreadVerifyError(err.to_string()))?;
            let mut request = session.client().build();
            request
                .get_thread()
                .ids([thread_id])
                .properties([hail_jmap::jmap_client::thread::Property::Id]);
            let mut response = request
                .send_get_thread()
                .await
                .map_err(|err| ThreadVerifyError(err.to_string()))?;
            Ok(!response.take_list().is_empty())
        })
    }
}

/// Opaque verifier error. Detailed JMAP errors stay in server logs only.
#[derive(Debug)]
pub struct ThreadVerifyError(String);

/// Build protected thread-verb routes.
pub fn router() -> Router<AppState> {
    router_with_verifier(Arc::new(JmapThreadVerifier))
}

/// Test/helper router that injects a fake thread verifier. Kept generic so
/// integration tests can exercise the real handler without Stalwart.
pub fn router_with_verifier<V>(verifier: Arc<V>) -> Router<AppState>
where
    V: ThreadVerifier,
{
    Router::new()
        .route(
            "/api/threads/{thread_id}/bubble-up",
            post(bubble_up::<V>),
        )
        .layer(Extension(verifier))
}

#[derive(Debug, Deserialize)]
struct BubbleUpRequest {
    at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct BubbleUpResponse {
    bubble_id: i64,
    surface_at: DateTime<Utc>,
}

async fn bubble_up<V>(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(verifier): Extension<Arc<V>>,
    Path(thread_id): Path<String>,
    Json(body): Json<BubbleUpRequest>,
) -> Response
where
    V: ThreadVerifier,
{
    if !looks_like_jmap_id(&thread_id) {
        return bad_request("invalid_thread_id");
    }

    if body.at < Utc::now() + Duration::seconds(60) {
        return bad_request("at_must_be_in_future");
    }

    let visible = match verifier.exists(&state, user.jmap_token.clone(), &thread_id).await {
        Ok(visible) => visible,
        Err(err) => {
            tracing::warn!(user_id = user.id, thread_id = %thread_id, error = %err.0, "thread visibility check failed");
            return internal();
        }
    };
    if !visible {
        return (StatusCode::NOT_FOUND, [(header::CONTENT_TYPE, "application/json")], r#"{"error":"not_found"}"#).into_response();
    }

    let now = Utc::now();
    let bubble_id = match sqlx::query_scalar::<_, i64>(
        "INSERT INTO bubble_ups (user_id, thread_id, surface_at, created_at) \
         VALUES (?1, ?2, ?3, ?4) RETURNING id",
    )
    .bind(user.id)
    .bind(&thread_id)
    .bind(body.at)
    .bind(now)
    .fetch_one(&state.db)
    .await
    {
        Ok(id) => id,
        Err(err) => {
            tracing::error!(user_id = user.id, thread_id = %thread_id, error = %err, "bubble-up insert failed");
            return internal();
        }
    };

    (StatusCode::CREATED, Json(BubbleUpResponse { bubble_id, surface_at: body.at })).into_response()
}

fn looks_like_jmap_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 256
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

fn bad_request(error: &'static str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        [(header::CONTENT_TYPE, "application/json")],
        format!(r#"{{"error":"{error}"}}"#),
    )
        .into_response()
}

fn internal() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"error":"internal"}"#,
    )
        .into_response()
}
