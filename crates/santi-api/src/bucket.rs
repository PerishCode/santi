use axum::{
    body::Body,
    extract::{Path, State},
    http::{StatusCode, header},
    response::Response,
};
use santi_core::{ErrorResponse, SantiService};

use crate::ApiError;

#[utoipa::path(
    get,
    path = "/api/v1/bucket/{soul_id}/{strand_id}/{key}",
    params(
        ("soul_id" = String, Path),
        ("strand_id" = String, Path),
        ("key" = String, Path)
    ),
    responses(
        (status = 200, description = "Bucket object bytes", content_type = "application/octet-stream"),
        (status = 400, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
        (status = 500, body = ErrorResponse)
    )
)]
pub(crate) async fn get_bucket_object(
    State(service): State<SantiService>,
    Path((soul_id, strand_id, key)): Path<(String, String, String)>,
) -> Result<Response, ApiError> {
    let payload = service
        .get_bucket_object(&soul_id, &strand_id, &key)
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
