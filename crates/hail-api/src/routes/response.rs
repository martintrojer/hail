use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
pub struct ApiError {
    pub error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

impl ApiError {
    pub fn new(error: impl Into<String>) -> Self {
        Self {
            error: error.into(),
            detail: None,
            code: None,
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }

    pub fn into_response(self, status: StatusCode) -> Response {
        (status, Json(self)).into_response()
    }
}

pub fn error_response(status: StatusCode, error: impl Into<String>) -> Response {
    ApiError::new(error).into_response(status)
}

pub fn bad_request(error: impl Into<String>) -> Response {
    error_response(StatusCode::BAD_REQUEST, error)
}

pub fn not_found(entity: &str) -> Response {
    error_response(StatusCode::NOT_FOUND, entity)
}

pub fn internal() -> Response {
    error_response(StatusCode::INTERNAL_SERVER_ERROR, "internal")
}
