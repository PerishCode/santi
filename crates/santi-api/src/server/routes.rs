use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use santi_core::{
    CompactExecRequest, CompactExecResponse, CompactQueryResponse, CreateSoulRequest,
    CreateStrandResponse, CreateWebhookRequest, DriveStrandResponse, ForkStrandResponse,
    HealthResponse, MaterialRequest, ReceiptStatus, SantiError, SantiService,
    SendStrandAcceptedResponse, SendStrandRequest, Soul, Strand, StrandBudgetSnapshot,
    StrandDetail, StrandMaterial, StrandRuntimeSnapshot, WebhookSubscription,
};
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};

use super::{
    ApiError,
    errors::{errors, strand_errors},
    ingress::ingest_webhook,
    sse::{error_events, strand_events},
};

pub(super) fn router(service: SantiService) -> Router {
    let api = Router::new()
        .route("/api/v1/openapi.json", get(openapi))
        .route("/api/v1/strands", post(create_strand).get(list_strands))
        .route("/api/v1/souls", post(create_soul).get(list_souls))
        .route("/api/v1/souls/{soul_id}", get(get_soul))
        .route("/api/v1/webhooks", post(create_webhook).get(list_webhooks))
        .route("/api/v1/strands/{strand_id}", get(get_strand))
        .route("/api/v1/strands/{strand_id}/messages", get(list_messages))
        .route(
            "/api/v1/strands/{strand_id}/materials",
            post(strand_material),
        )
        .route("/api/v1/strands/{strand_id}/events", get(strand_events))
        .route("/api/v1/strands/{strand_id}/send", post(send_strand))
        .route("/api/v1/strands/{strand_id}/drive", post(drive_strand))
        .route("/api/v1/strands/{strand_id}/fork", post(fork_strand))
        .route("/api/v1/strands/{strand_id}/compact", post(compact_exec))
        .route("/api/v1/strands/{strand_id}/budget", get(strand_budget))
        .route("/api/v1/strands/{strand_id}/errors", get(strand_errors))
        .route("/api/v1/errors/events", get(error_events))
        .route("/api/v1/errors/{scope_kind}/{scope_id}", get(errors))
        .route("/api/v1/receipts/{inbox_id}", get(receipt_status))
        .route(
            "/api/v1/effects/{effect_id}",
            get(super::effects::effect_status),
        )
        .route(
            "/api/v1/effects/{effect_id}/resolve",
            post(super::effects::resolve_effect),
        )
        .route("/api/v1/compacts/{compact_id}", get(compact_query))
        .route("/api/v1/strands/{strand_id}/runtime", get(runtime_snapshot))
        // IM layer (orthogonal to the runtime; shares the server for cold-start):
        // send into a soul's IM conversation, poll a participant's passive inbox.
        .route("/api/v1/im/send", post(super::im::send_im))
        .route("/api/v1/im/inbox/{participant_id}", get(super::im::poll_im))
        .route(
            "/api/v1/bucket/{soul_id}/{strand_id}/{*key}",
            get(crate::bucket::get_bucket_object),
        );

    Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/webhooks/{name}", post(ingest_webhook))
        .merge(api)
        .layer(TraceLayer::new_for_http())
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .with_state(service)
}

#[utoipa::path(
    get,
    path = "/api/v1/health",
    responses(
        (status = 200, body = HealthResponse),
        (status = 503, body = HealthResponse)
    )
)]
pub async fn health(State(service): State<SantiService>) -> impl IntoResponse {
    let active_drive_incidents = service.active_drive_incident_count();
    let degraded = service.is_drive_degraded() || active_drive_incidents > 0;
    let status = if degraded {
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        StatusCode::OK
    };
    (
        status,
        Json(HealthResponse {
            ok: !degraded,
            degraded,
            service: "santi-api".to_string(),
            active_drive_incidents,
        }),
    )
}

#[utoipa::path(
    get,
    path = "/api/v1/receipts/{inbox_id}",
    params(("inbox_id" = String, Path)),
    responses(
        (status = 200, body = ReceiptStatus),
        (status = 404, body = SantiError),
        (status = 500, body = SantiError)
    )
)]
pub async fn receipt_status(
    State(service): State<SantiService>,
    Path(inbox_id): Path<String>,
) -> Result<Json<ReceiptStatus>, ApiError> {
    service
        .receipt_status(&inbox_id)
        .map_err(ApiError::from_service)?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("receipt not found"))
}

#[utoipa::path(
    post,
    path = "/api/v1/strands",
    responses((status = 200, body = CreateStrandResponse), (status = 500, body = SantiError))
)]
pub(super) async fn create_strand(
    State(service): State<SantiService>,
) -> Result<Json<CreateStrandResponse>, ApiError> {
    service
        .create_strand()
        .map(Json)
        .map_err(ApiError::from_service)
}

#[utoipa::path(
    get,
    path = "/api/v1/strands",
    responses((status = 200, body = [Strand]), (status = 500, body = SantiError))
)]
pub(super) async fn list_strands(
    State(service): State<SantiService>,
) -> Result<Json<Vec<Strand>>, ApiError> {
    service
        .list_strands()
        .map(Json)
        .map_err(ApiError::from_service)
}

#[utoipa::path(
    post,
    path = "/api/v1/souls",
    request_body = CreateSoulRequest,
    responses((status = 200, body = Soul), (status = 500, body = SantiError))
)]
pub(super) async fn create_soul(
    State(service): State<SantiService>,
    Json(request): Json<CreateSoulRequest>,
) -> Result<Json<Soul>, ApiError> {
    service
        .create_soul(request)
        .map(Json)
        .map_err(ApiError::from_service)
}

#[utoipa::path(
    get,
    path = "/api/v1/souls",
    responses((status = 200, body = [Soul]), (status = 500, body = SantiError))
)]
pub(super) async fn list_souls(
    State(service): State<SantiService>,
) -> Result<Json<Vec<Soul>>, ApiError> {
    service
        .list_souls()
        .map(Json)
        .map_err(ApiError::from_service)
}

#[utoipa::path(
    get,
    path = "/api/v1/souls/{soul_id}",
    params(("soul_id" = String, Path)),
    responses(
        (status = 200, body = Soul),
        (status = 404, body = SantiError),
        (status = 500, body = SantiError)
    )
)]
pub(super) async fn get_soul(
    State(service): State<SantiService>,
    Path(soul_id): Path<String>,
) -> Result<Json<Soul>, ApiError> {
    match service.soul(&soul_id).map_err(ApiError::from_service)? {
        Some(soul) => Ok(Json(soul)),
        None => Err(ApiError::not_found("soul not found")),
    }
}

#[utoipa::path(
    post,
    path = "/api/v1/webhooks",
    request_body = CreateWebhookRequest,
    responses((status = 200, body = WebhookSubscription), (status = 500, body = SantiError))
)]
pub(super) async fn create_webhook(
    State(service): State<SantiService>,
    Json(request): Json<CreateWebhookRequest>,
) -> Result<Json<WebhookSubscription>, ApiError> {
    service
        .create_webhook(request)
        .map(Json)
        .map_err(ApiError::from_service)
}

#[utoipa::path(
    get,
    path = "/api/v1/webhooks",
    responses((status = 200, body = [WebhookSubscription]), (status = 500, body = SantiError))
)]
pub(super) async fn list_webhooks(
    State(service): State<SantiService>,
) -> Result<Json<Vec<WebhookSubscription>>, ApiError> {
    service
        .list_webhooks()
        .map(Json)
        .map_err(ApiError::from_service)
}

#[utoipa::path(
    get,
    path = "/api/v1/strands/{strand_id}",
    params(("strand_id" = String, Path)),
    responses(
        (status = 200, body = StrandDetail),
        (status = 404, body = SantiError),
        (status = 500, body = SantiError)
    )
)]
pub(super) async fn get_strand(
    State(service): State<SantiService>,
    Path(strand_id): Path<String>,
) -> Result<Json<StrandDetail>, ApiError> {
    service
        .strand(&strand_id)
        .map_err(ApiError::from_service)?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("strand not found"))
}

#[utoipa::path(
    get,
    path = "/api/v1/strands/{strand_id}/messages",
    params(("strand_id" = String, Path)),
    responses(
        (status = 200, body = [santi_core::StrandMessage]),
        (status = 404, body = SantiError),
        (status = 500, body = SantiError)
    )
)]
pub(super) async fn list_messages(
    State(service): State<SantiService>,
    Path(strand_id): Path<String>,
) -> Result<Json<Vec<santi_core::StrandMessage>>, ApiError> {
    service
        .strand(&strand_id)
        .map_err(ApiError::from_service)?
        .map(|detail| Json(detail.messages))
        .ok_or_else(|| ApiError::not_found("strand not found"))
}

#[utoipa::path(
    post,
    path = "/api/v1/strands/{strand_id}/materials",
    params(("strand_id" = String, Path)),
    request_body = MaterialRequest,
    responses(
        (status = 200, body = StrandMaterial),
        (status = 404, body = SantiError),
        (status = 500, body = SantiError)
    )
)]
pub(super) async fn strand_material(
    State(service): State<SantiService>,
    Path(strand_id): Path<String>,
    Json(request): Json<MaterialRequest>,
) -> Result<Json<StrandMaterial>, ApiError> {
    service
        .strand_material(&strand_id, request)
        .map(Json)
        .map_err(ApiError::from_service)
}

#[utoipa::path(
    post,
    path = "/api/v1/strands/{strand_id}/send",
    params(("strand_id" = String, Path)),
    request_body = SendStrandRequest,
    responses(
        (status = 200, body = SendStrandAcceptedResponse),
        (status = 423, body = SantiError),
        (status = 404, body = SantiError),
        (status = 500, body = SantiError),
        (status = 503, body = SantiError)
    )
)]
pub async fn send_strand(
    State(service): State<SantiService>,
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
        (status = 404, body = SantiError),
        (status = 423, body = SantiError),
        (status = 500, body = SantiError),
        (status = 503, body = SantiError)
    )
)]
pub async fn drive_strand(
    State(service): State<SantiService>,
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
        (status = 404, body = SantiError),
        (status = 500, body = SantiError)
    )
)]
pub(super) async fn fork_strand(
    State(service): State<SantiService>,
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
        (status = 400, body = SantiError),
        (status = 404, body = SantiError),
        (status = 500, body = SantiError)
    )
)]
pub(super) async fn compact_exec(
    State(service): State<SantiService>,
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
        (status = 404, body = SantiError),
        (status = 500, body = SantiError)
    )
)]
pub(super) async fn compact_query(
    State(service): State<SantiService>,
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
        (status = 404, body = SantiError),
        (status = 500, body = SantiError)
    )
)]
pub(super) async fn runtime_snapshot(
    State(service): State<SantiService>,
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
        (status = 404, body = SantiError),
        (status = 500, body = SantiError)
    )
)]
pub(super) async fn strand_budget(
    State(service): State<SantiService>,
    Path(strand_id): Path<String>,
) -> Result<Json<StrandBudgetSnapshot>, ApiError> {
    service
        .strand_budget(&strand_id)
        .map_err(ApiError::from_service)?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("strand not found"))
}

pub(super) async fn openapi() -> Json<utoipa::openapi::OpenApi> {
    Json(super::openapi::document())
}
