use axum::{
    Json,
    body::Bytes,
    extract::{Query, State},
    http::HeaderMap,
};
use santi_core::service::Service;
use santi_core::service::window::Outcome;
use santi_core::{SantiError, WindowSendAccepted, WindowSendRequest, WindowTranscript, catalog};
use serde::Deserialize;

use super::ApiError;

const IDENTITY_HEADER: &str = "x-authentik-uid";

fn identity(headers: &HeaderMap) -> Result<String, Box<ApiError>> {
    let uid = headers
        .get(IDENTITY_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .unwrap_or("");
    if uid.is_empty() {
        return Err(Box::new(ApiError::from_santi(
            santi_core::engine().transient(santi_core::Signal {
                descriptor: catalog::WINDOW_IDENTITY_MISSING,
                source: santi_core::ErrorSource::new("santi-api", "window_boundary"),
                scope: None,
                message: "window identity header is missing or blank".to_string(),
                context: serde_json::Value::Null,
            }),
        )));
    }
    Ok(uid.to_string())
}

fn invalid(message: String) -> Box<ApiError> {
    Box::new(ApiError::from_santi(santi_core::engine().transient(
        santi_core::Signal {
            descriptor: catalog::WINDOW_CONTENT_INVALID,
            source: santi_core::ErrorSource::new("santi-api", "window_boundary"),
            scope: None,
            message,
            context: serde_json::Value::Null,
        },
    )))
}

fn parse_send(body: &Bytes) -> Result<WindowSendRequest, Box<ApiError>> {
    let value: serde_json::Value = serde_json::from_slice(body)
        .map_err(|error| invalid(format!("body is not valid JSON: {error}")))?;
    let object = value
        .as_object()
        .ok_or_else(|| invalid("body must be a JSON object".to_string()))?;
    let content = object
        .get("content")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| invalid("content must be a JSON string".to_string()))?;
    let client_message_id = object
        .get("client_message_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| invalid("client_message_id must be a JSON string".to_string()))?;
    Ok(WindowSendRequest {
        content: content.to_string(),
        client_message_id: client_message_id.to_string(),
    })
}

#[utoipa::path(
    post,
    path = "/api/v1/window/im/send",
    request_body = WindowSendRequest,
    responses(
        (status = 200, body = WindowSendAccepted),
        (status = 400, body = SantiError),
        (status = 401, body = SantiError),
        (status = 409, body = SantiError),
        (status = 413, body = SantiError),
        (status = 429, body = SantiError),
        (status = 500, body = SantiError)
    )
)]
pub async fn send_window(
    State(service): State<Service>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<WindowSendAccepted>, ApiError> {
    let uid = identity(&headers).map_err(|boxed| *boxed)?;
    let request = parse_send(&body).map_err(|boxed| *boxed)?;
    match service
        .window_send(&uid, request)
        .map_err(ApiError::from_service)?
    {
        Outcome::Accepted(accepted) => Ok(Json(accepted)),
        Outcome::Rejected(error) => Err(ApiError::from_santi(*error)),
    }
}

#[derive(Debug, Deserialize)]
pub struct TranscriptQuery {
    #[serde(default)]
    since: i64,
    limit: Option<usize>,
}

#[utoipa::path(
    get,
    path = "/api/v1/window/im/transcript",
    params(
        ("since" = Option<i64>, Query, description = "Exclusive strand sequence cursor"),
        ("limit" = Option<usize>, Query, description = "Page size, 1..=200 (default 200)")
    ),
    responses(
        (status = 200, body = WindowTranscript),
        (status = 401, body = SantiError),
        (status = 500, body = SantiError)
    )
)]
pub async fn transcript_window(
    State(service): State<Service>,
    headers: HeaderMap,
    Query(query): Query<TranscriptQuery>,
) -> Result<Json<WindowTranscript>, ApiError> {
    let uid = identity(&headers).map_err(|boxed| *boxed)?;
    let transcript = service
        .window_transcript(&uid, query.since.max(0), query.limit.unwrap_or(200))
        .map_err(ApiError::from_service)?;
    Ok(Json(transcript))
}
