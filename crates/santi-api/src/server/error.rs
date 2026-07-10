use axum::{Json, http::StatusCode, response::IntoResponse};
use santi_core::{ErrorCategory, ErrorSource, SantiError, catalog, engine};

use crate::webhook::WebhookError;

pub struct ApiError {
    status: StatusCode,
    error: SantiError,
}

impl ApiError {
    pub fn status(&self) -> StatusCode {
        self.status
    }

    pub fn code(&self) -> &str {
        &self.error.code
    }

    pub fn message(&self) -> &str {
        &self.error.message
    }

    pub fn internal(message: String) -> Self {
        eprintln!("santi-api: internal error: {message}");
        Self::from_santi(engine().transient(
            catalog::INTERNAL,
            ErrorSource::new("santi-api", "http_boundary"),
            None,
            "internal error",
            serde_json::Value::Null,
        ))
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::from_santi(engine().transient(
            catalog::NOT_FOUND,
            ErrorSource::new("santi-api", "http_boundary"),
            None,
            message,
            serde_json::Value::Null,
        ))
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::from_santi(engine().transient(
            catalog::INVALID_ARGUMENT,
            ErrorSource::new("santi-api", "http_boundary"),
            None,
            message,
            serde_json::Value::Null,
        ))
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::from_santi(engine().transient(
            catalog::UNAUTHORIZED,
            ErrorSource::new("santi-api", "http_boundary"),
            None,
            message,
            serde_json::Value::Null,
        ))
    }

    pub fn from_santi(error: SantiError) -> Self {
        let status = if error.code == catalog::CONTEXT_BUDGET_EXCEEDED.code {
            StatusCode::LOCKED
        } else {
            match error.category {
                ErrorCategory::Internal => StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCategory::InvalidInput => StatusCode::BAD_REQUEST,
                ErrorCategory::NotFound => StatusCode::NOT_FOUND,
                ErrorCategory::ResourceExhausted => StatusCode::TOO_MANY_REQUESTS,
                ErrorCategory::Unauthorized => StatusCode::UNAUTHORIZED,
                ErrorCategory::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            }
        };
        Self { status, error }
    }

    pub fn from_webhook(error: WebhookError) -> Self {
        match error {
            WebhookError::Unauthorized(message) => Self::unauthorized(message),
            WebhookError::BadRequest(message) => Self::bad_request(message),
        }
    }

    pub fn from_service(message: String) -> Self {
        let text = message.as_str();
        if text == "strand not found" || text == "soul not found" || text.ends_with("not found") {
            Self::not_found(message)
        } else if text.starts_with("unknown soul")
            || text.contains("must not be empty")
            || text.contains("must contain text")
            || text.starts_with("strand_strategy must be")
            || text.starts_with("fork_point")
            || text.contains(" is past parent end ")
            || text.contains("object key")
            || text.contains("object uri")
            || text.contains("path segment")
            || text.contains("path separators")
        {
            Self::bad_request(message)
        } else {
            Self::internal(message)
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (self.status, Json(self.error)).into_response()
    }
}
