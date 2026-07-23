use super::*;

#[utoipa::path(
    post,
    path = "/api/v1/strands/{strand_id}/send",
    params(("strand_id" = String, Path)),
    request_body = SendStrandRequest,
    responses(
        (status = 200, body = SendStrandAcceptedResponse),
        (status = 423, body = Fault),
        (status = 404, body = Fault),
        (status = 500, body = Fault),
        (status = 503, body = Fault)
    )
)]
pub async fn send_strand(
    State(service): State<Service>,
    Path(strand_id): Path<String>,
    Json(request): Json<SendStrandRequest>,
) -> Result<Json<SendStrandAcceptedResponse>, ApiError> {
    service
        .send_strand(&strand_id, request)
        .await
        .map(Json)
        .map_err(ApiError::from_santi)
}

#[utoipa::path(
    post,
    path = "/api/v1/strands/{strand_id}/drive",
    params(("strand_id" = String, Path)),
    responses(
        (status = 200, body = DriveStrandResponse),
        (status = 404, body = Fault),
        (status = 423, body = Fault),
        (status = 500, body = Fault),
        (status = 503, body = Fault)
    )
)]
pub async fn drive_strand(
    State(service): State<Service>,
    Path(strand_id): Path<String>,
) -> Result<Json<DriveStrandResponse>, ApiError> {
    service
        .drive_strand(&strand_id)
        .map(Json)
        .map_err(|error| ApiError::from_santi(*error))
}

#[utoipa::path(
    post,
    path = "/api/v1/strands/{strand_id}/fork",
    params(("strand_id" = String, Path)),
    responses(
        (status = 200, body = ForkStrandResponse),
        (status = 404, body = Fault),
        (status = 500, body = Fault)
    )
)]
pub(super) async fn fork_strand(
    State(service): State<Service>,
    Path(strand_id): Path<String>,
) -> Result<Json<ForkStrandResponse>, ApiError> {
    service
        .fork_strand(&strand_id)
        .map(Json)
        .map_err(ApiError::from_service)
}

#[utoipa::path(
    post,
    path = "/api/v1/strands/{strand_id}/compact",
    params(("strand_id" = String, Path)),
    request_body = CompactExecRequest,
    responses(
        (status = 200, body = CompactExecResponse),
        (status = 400, body = Fault),
        (status = 404, body = Fault),
        (status = 500, body = Fault)
    )
)]
pub(super) async fn compact_exec(
    State(service): State<Service>,
    Path(strand_id): Path<String>,
    Json(request): Json<CompactExecRequest>,
) -> Result<Json<CompactExecResponse>, ApiError> {
    service
        .compact_exec(&strand_id, request)
        .map(Json)
        .map_err(ApiError::from_service)
}

#[utoipa::path(
    get,
    path = "/api/v1/compacts/{compact_id}",
    params(
        ("compact_id" = String, Path),
        ("keyword" = Option<String>, Query),
        ("page_index" = Option<i64>, Query),
        ("page_size" = Option<i64>, Query)
    ),
    responses(
        (status = 200, body = CompactQueryResponse),
        (status = 404, body = Fault),
        (status = 500, body = Fault)
    )
)]
pub(super) async fn compact_query(
    State(service): State<Service>,
    Path(compact_id): Path<String>,
    Query(params): Query<CompactQueryParams>,
) -> Result<Json<CompactQueryResponse>, ApiError> {
    service
        .compact_query(
            &compact_id,
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
    path = "/api/v1/strands/{strand_id}/runtime",
    params(("strand_id" = String, Path)),
    responses(
        (status = 200, body = StrandRuntimeSnapshot),
        (status = 404, body = Fault),
        (status = 500, body = Fault)
    )
)]
pub(super) async fn runtime_snapshot(
    State(service): State<Service>,
    Path(strand_id): Path<String>,
) -> Result<Json<StrandRuntimeSnapshot>, ApiError> {
    service
        .runtime_snapshot(&strand_id)
        .map_err(ApiError::from_service)?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("strand not found"))
}

#[utoipa::path(
    get,
    path = "/api/v1/strands/{strand_id}/budget",
    params(("strand_id" = String, Path)),
    responses(
        (status = 200, body = StrandBudgetSnapshot),
        (status = 404, body = Fault),
        (status = 500, body = Fault)
    )
)]
pub(super) async fn strand_budget(
    State(service): State<Service>,
    Path(strand_id): Path<String>,
) -> Result<Json<StrandBudgetSnapshot>, ApiError> {
    service
        .strand_budget(&strand_id)
        .map_err(ApiError::from_service)?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("strand not found"))
}

pub(super) async fn openapi() -> Json<utoipa::openapi::OpenApi> {
    Json(crate::server::openapi::document())
}
