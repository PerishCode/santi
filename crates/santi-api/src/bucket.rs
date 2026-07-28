use axum::{
    body::Body,
    extract::{Path, State},
    http::{StatusCode, header},
    response::Response,
};
use santi_core::Fault;
use santi_core::service::Service;

use crate::ApiError;

#[utoipa::path(
    get,
    path = "/api/v1/bucket/{soul}/{strand}/{key}",
    params(
        ("soul" = String, Path),
        ("strand" = String, Path),
        ("key" = String, Path)
    ),
    responses(
        (status = 200, description = "Bucket object bytes", content_type = "application/octet-stream"),
        (status = 400, body = Fault),
        (status = 404, body = Fault),
        (status = 500, body = Fault)
    )
)]
pub(crate) async fn fetch(
    State(service): State<Service>,
    Path((soul, strand, key)): Path<(String, String, String)>,
) -> Result<Response, ApiError> {
    let payload = service
        .fetch(&soul, &strand, &key)
        .await
        .map_err(ApiError::from_service)?
        .ok_or_else(|| ApiError::not_found("object not found"))?;
    Response::builder()
        .status(StatusCode::OK)
        .header(
            header::CONTENT_TYPE,
            content_type_for_key(&payload.meta.uri.key),
        )
        .header(header::CONTENT_LENGTH, payload.meta.len.to_string())
        .body(Body::from(payload.bytes))
        .map_err(|error| ApiError::internal(error.to_string()))
}

fn content_type_for_key(key: &str) -> &'static str {
    match key.rsplit('.').next().unwrap_or_default() {
        "css" => "text/css; charset=utf-8",
        "gif" => "image/gif",
        "htm" | "html" => "text/html; charset=utf-8",
        "jpeg" | "jpg" => "image/jpeg",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "png" => "image/png",
        "svg" => "image/svg+xml",
        "txt" => "text/plain; charset=utf-8",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    }
}
