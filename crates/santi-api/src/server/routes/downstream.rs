use axum::http::HeaderMap;
use santi_core::service::Admission;
use santi_core::{IngestOutcome, IngestReceipt};

use super::*;

#[utoipa::path(
    post,
    path = "/api/v1/downstreams",
    responses(
        (status = 200, body = DownstreamCredential),
        (status = 400, body = Fault),
        (status = 409, body = Fault),
        (status = 500, body = Fault)
    )
)]
pub(super) async fn create_downstream(
    State(service): State<Service>,
    Json(request): Json<CreateDownstreamRequest>,
) -> Result<Json<DownstreamCredential>, ApiError> {
    service
        .create_downstream(request)
        .map(Json)
        .map_err(ApiError::from_service)
}

#[utoipa::path(
    get,
    path = "/api/v1/downstreams",
    responses(
        (status = 200, body = Vec<DownstreamCredential>),
        (status = 500, body = Fault)
    )
)]
pub(super) async fn list_downstreams(
    State(service): State<Service>,
) -> Result<Json<Vec<DownstreamCredential>>, ApiError> {
    service
        .list_downstreams()
        .map(Json)
        .map_err(ApiError::from_service)
}

#[utoipa::path(
    post,
    path = "/api/v1/ingest",
    security(("downstream_bearer" = [])),
    request_body = IngestRequest,
    responses(
        (status = 202, body = IngestReceipt),
        (status = 400, body = Fault),
        (status = 401, body = Fault),
        (status = 403, body = Fault),
        (status = 409, body = Fault),
        (status = 500, body = Fault)
    )
)]
pub(super) async fn ingest(
    State(service): State<Service>,
    headers: HeaderMap,
    Json(request): Json<IngestRequest>,
) -> Result<Json<IngestReceipt>, ApiError> {
    let token = bearer(&headers);
    match service
        .ingest_downstream(token, request)
        .map_err(ApiError::from_service)?
    {
        Admission::Accepted(IngestOutcome::Accepted { receipt }) => Ok(Json(receipt)),
        Admission::Accepted(IngestOutcome::Rejected { error }) => Err(ApiError::from_santi(*error)),
        Admission::Denied => Err(ApiError::unauthorized("invalid or missing credential")),
        Admission::Forbidden => Err(ApiError::forbidden(
            "label outside the credential's authorized prefix",
        )),
    }
}

pub(crate) fn bearer(headers: &HeaderMap) -> &str {
    headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or("")
}
