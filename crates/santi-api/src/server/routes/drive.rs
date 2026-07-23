use super::*;
use santi_core::{budget, compact, strand, stream};

#[utoipa::path(
    post,
    path = "/api/v1/strands/{strand}/send",
    params(("strand" = String, Path)),
    request_body = strand::Post,
    responses(
        (status = 200, body = strand::Posted),
        (status = 423, body = Fault),
        (status = 404, body = Fault),
        (status = 500, body = Fault),
        (status = 503, body = Fault)
    )
)]
pub async fn send(
    State(service): State<Service>,
    Path(strand): Path<String>,
    Json(request): Json<strand::Post>,
) -> Result<Json<strand::Posted>, ApiError> {
    service
        .send(&strand, request)
        .await
        .map(Json)
        .map_err(ApiError::from_santi)
}

#[utoipa::path(
    post,
    path = "/api/v1/strands/{strand}/drive",
    params(("strand" = String, Path)),
    responses(
        (status = 200, body = drive::Response),
        (status = 404, body = Fault),
        (status = 423, body = Fault),
        (status = 500, body = Fault),
        (status = 503, body = Fault)
    )
)]
pub async fn drive(
    State(service): State<Service>,
    Path(strand): Path<String>,
) -> Result<Json<drive::Response>, ApiError> {
    service
        .drive(&strand)
        .map(Json)
        .map_err(|error| ApiError::from_santi(*error))
}

#[utoipa::path(
    post,
    path = "/api/v1/strands/{strand}/fork",
    params(("strand" = String, Path)),
    responses(
        (status = 200, body = strand::Forked),
        (status = 404, body = Fault),
        (status = 500, body = Fault)
    )
)]
pub(super) async fn fork(
    State(service): State<Service>,
    Path(strand): Path<String>,
) -> Result<Json<strand::Forked>, ApiError> {
    service
        .fork(&strand)
        .map(Json)
        .map_err(ApiError::from_service)
}

#[utoipa::path(
    post,
    path = "/api/v1/strands/{strand}/compact",
    params(("strand" = String, Path)),
    request_body = compact::Exec,
    responses(
        (status = 200, body = compact::Report),
        (status = 400, body = Fault),
        (status = 404, body = Fault),
        (status = 500, body = Fault)
    )
)]
pub(super) async fn exec(
    State(service): State<Service>,
    Path(strand): Path<String>,
    Json(request): Json<compact::Exec>,
) -> Result<Json<compact::Report>, ApiError> {
    service
        .exec(&strand, request)
        .map(Json)
        .map_err(ApiError::from_service)
}

#[utoipa::path(
    get,
    path = "/api/v1/compacts/{compact}",
    params(
        ("compact" = String, Path),
        ("keyword" = Option<String>, Query),
        ("page_index" = Option<i64>, Query),
        ("page_size" = Option<i64>, Query)
    ),
    responses(
        (status = 200, body = compact::Page),
        (status = 404, body = Fault),
        (status = 500, body = Fault)
    )
)]
pub(super) async fn page(
    State(service): State<Service>,
    Path(compact): Path<String>,
    Query(params): Query<CompactQueryParams>,
) -> Result<Json<compact::Page>, ApiError> {
    service
        .page(
            &compact,
            params.keyword.as_deref(),
            params.page_index.unwrap_or(0),
            params.page_size.unwrap_or(50),
        )
        .map_err(ApiError::from_service)?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("compact not found"))
}

#[derive(serde::Deserialize)]
pub(super) struct CompactQueryParams {
    keyword: Option<String>,
    page_index: Option<i64>,
    page_size: Option<i64>,
}

#[utoipa::path(
    get,
    path = "/api/v1/strands/{strand}/runtime",
    params(("strand" = String, Path)),
    responses(
        (status = 200, body = stream::Snapshot),
        (status = 404, body = Fault),
        (status = 500, body = Fault)
    )
)]
pub(super) async fn snapshot(
    State(service): State<Service>,
    Path(strand): Path<String>,
) -> Result<Json<stream::Snapshot>, ApiError> {
    service
        .snapshot(&strand)
        .map_err(ApiError::from_service)?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("strand not found"))
}

#[utoipa::path(
    get,
    path = "/api/v1/strands/{strand}/budget",
    params(("strand" = String, Path)),
    responses(
        (status = 200, body = budget::Snapshot),
        (status = 404, body = Fault),
        (status = 500, body = Fault)
    )
)]
pub(super) async fn audit(
    State(service): State<Service>,
    Path(strand): Path<String>,
) -> Result<Json<budget::Snapshot>, ApiError> {
    service
        .audit(&strand)
        .map_err(ApiError::from_service)?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("strand not found"))
}

pub(super) async fn openapi() -> Json<utoipa::openapi::OpenApi> {
    Json(crate::server::openapi::document())
}
