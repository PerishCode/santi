use axum::{Json, http::StatusCode, response::IntoResponse};
use santi_core::{Category, Fault, Ruled, Signal, budget, catalog, engine};

use crate::webhook::WebhookError;

pub struct ApiError {
    status: StatusCode,
    error: Fault,
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
            source: santi_core::Source::new("santi-api", "http_boundary"),
            scope: None,
            message: "internal error".to_string(),
            context: serde_json::Value::Null,
        }))
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::from_santi(engine().transient(Signal {
            descriptor: catalog::NOT_FOUND,
            source: santi_core::Source::new("santi-api", "http_boundary"),
            scope: None,
            message: message.into(),
            context: serde_json::Value::Null,
        }))
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::from_santi(engine().transient(Signal {
            descriptor: catalog::INVALID_ARGUMENT,
            source: santi_core::Source::new("santi-api", "http_boundary"),
            scope: None,
            message: message.into(),
            context: serde_json::Value::Null,
        }))
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::from_santi(engine().transient(Signal {
            descriptor: catalog::UNAUTHORIZED,
            source: santi_core::Source::new("santi-api", "http_boundary"),
            scope: None,
            message: message.into(),
            context: serde_json::Value::Null,
        }))
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        let mut error = Self::unauthorized(message);
        error.status = StatusCode::FORBIDDEN;
        error
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::from_santi(engine().transient(Signal {
            descriptor: catalog::UNAVAILABLE,
            source: santi_core::Source::new("santi-api", "http_boundary"),
            scope: None,
            message: message.into(),
            context: serde_json::Value::Null,
        }))
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        let mut error = Self::bad_request(message);
        error.status = StatusCode::CONFLICT;
        error
    }

    pub fn from_santi(error: Fault) -> Self {
        let status = if error.code == budget::Error::Context.descriptor().code {
            StatusCode::LOCKED
        } else {
            match error.category {
                Category::Internal => StatusCode::INTERNAL_SERVER_ERROR,
                Category::Invalid => StatusCode::BAD_REQUEST,
                Category::Missing => StatusCode::NOT_FOUND,
                Category::Exhausted => StatusCode::TOO_MANY_REQUESTS,
                Category::Unauthorized => StatusCode::UNAUTHORIZED,
                Category::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
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
        if text == "invalid job capability" || text == "job capability expired" {
            Self::unauthorized(message)
        } else if text.starts_with("job supervisor is unavailable")
            || text.starts_with("systemd did not accept job")
            || text.starts_with("launchd did not accept job")
            || text.contains("sidecar did not claim")
            || text.contains("sidecar failed before claimed")
        {
            Self::unavailable(message)
        } else if text == "strand not found"
            || text == "soul not found"
            || text.ends_with("not found")
        {
            Self::not_found(message)
        } else if text.starts_with("downstream request conflicts")
            || text.starts_with("downstream id conflicts")
            || text.starts_with("webhook delivery conflicts")
            || text.starts_with("webhook ") && text.contains(" conflicts ")
            || text.starts_with("job capability conflicts")
            || text.starts_with("job execution spec conflicts")
            || text.contains("sidecar stamp conflicts")
            || text.contains("overlaps an existing registration")
            || text.ends_with("is already registered")
        {
            Self::conflict(message)
        } else if text.starts_with("unknown soul")
            || text.contains("must not be empty")
            || text.starts_with("downstream ") && text.contains(" must ")
            || text.contains("must contain text")
            || text.starts_with("strategy must be")
            || text.starts_with("fork")
            || text.contains(" is past parent end ")
            || text.contains("object key")
            || text.contains("object uri")
            || text.contains("path segment")
            || text.contains("path separators")
            || text.starts_with("only an unknown effect")
            || text.starts_with("effect resolution evidence")
            || text.starts_with("job ") && text.contains(" must ")
            || text.starts_with("environment ")
            || text.starts_with("only a terminal job")
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
