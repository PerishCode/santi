use axum::{Json, http::StatusCode, response::IntoResponse};
use santi_core::ErrorResponse;

use crate::webhook::WebhookError;

pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl ApiError {
    pub fn status(&self) -> StatusCode {
        self.status
    }

    pub fn code(&self) -> &'static str {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn internal(message: String) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "internal",
            message,
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: "not-found",
            message: message.into(),
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "bad-request",
            message: message.into(),
        }
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            code: "unauthorized",
            message: message.into(),
        }
    }

    pub fn locked(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::LOCKED,
            code: "strand-blocked",
            message: message.into(),
        }
    }

    pub fn from_webhook(error: WebhookError) -> Self {
        match error {
            WebhookError::Unauthorized(message) => Self::unauthorized(message),
            WebhookError::BadRequest(message) => Self::bad_request(message),
        }
    }

    /// Classify a `santi-core` service error (a plain message) into an HTTP
    /// status. Referenced-but-absent resources are 404; rejected input is 400;
    /// broken invariants ("… disappeared") stay 500. Unrecognized messages
    /// degrade to 500, so routing any service error through here is safe.
    pub fn from_service(message: String) -> Self {
        let text = message.as_str();
        if text == "strand not found" || text == "soul not found" || text.ends_with("not found") {
            Self::not_found(message)
        } else if text.starts_with("strand is blocked: context_over_budget")
            || text.starts_with("strand context is over budget")
        {
            Self::locked(message)
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
            || text.starts_with("strand inbox is full")
        {
            Self::bad_request(message)
        } else {
            Self::internal(message)
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (
            self.status,
            Json(ErrorResponse {
                code: self.code.to_string(),
                message: self.message,
            }),
        )
            .into_response()
    }
}
