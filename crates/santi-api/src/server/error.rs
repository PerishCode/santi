use axum::{Json, http::StatusCode, response::IntoResponse};
use santi_core::{ErrorCategory, ErrorSource, SantiError, Signal, catalog, engine};

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
        Self::from_santi(engine().transient(Signal {
            descriptor: catalog::INTERNAL,
            source: ErrorSource::new("santi-api", "http_boundary"),
            scope: None,
            message: "internal error".to_string(),
            context: serde_json::Value::Null,
        }))
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::from_santi(engine().transient(Signal {
            descriptor: catalog::NOT_FOUND,
            source: ErrorSource::new("santi-api", "http_boundary"),
            scope: None,
            message: message.into(),
            context: serde_json::Value::Null,
        }))
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::from_santi(engine().transient(Signal {
            descriptor: catalog::INVALID_ARGUMENT,
            source: ErrorSource::new("santi-api", "http_boundary"),
            scope: None,
            message: message.into(),
            context: serde_json::Value::Null,
        }))
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::from_santi(engine().transient(Signal {
            descriptor: catalog::UNAUTHORIZED,
            source: ErrorSource::new("santi-api", "http_boundary"),
            scope: None,
            message: message.into(),
            context: serde_json::Value::Null,
        }))
    }

    pub fn from_santi(error: SantiError) -> Self {
        let status = if error.code == catalog::CONTEXT_BUDGET_EXCEEDED.code {
            StatusCode::LOCKED
        } else if error.code == catalog::WINDOW_MESSAGE_CONFLICT.code {
            StatusCode::CONFLICT
        } else if error.code == catalog::WINDOW_CONTENT_OVERSIZE.code {
            StatusCode::PAYLOAD_TOO_LARGE
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
            || text.starts_with("only an unknown effect")
            || text.starts_with("effect resolution evidence")
        {
            Self::bad_request(message)
        } else {
            Self::internal(message)
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let retry_after = self
            .error
            .context
            .get("retry_after_seconds")
            .and_then(serde_json::Value::as_u64);
        let mut response = (self.status, Json(self.error)).into_response();
        if let Some(seconds) = retry_after
            && let Ok(value) = axum::http::HeaderValue::from_str(&seconds.to_string())
        {
            response.headers_mut().insert("retry-after", value);
        }
        response
    }
}
