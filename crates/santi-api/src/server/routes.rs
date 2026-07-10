use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, post},
};
use santi_core::{
    CompactExecRequest, CompactExecResponse, CompactQueryResponse, CreateSoulRequest,
    CreateStrandResponse, CreateWebhookRequest, ErrorResponse, ForkStrandResponse, HealthResponse,
    ImInboxEntry, ImSendRequest, ImSendResponse, IngestOutcome, MaterialRequest, RejectedDelivery,
    SantiService, SendStrandAcceptedResponse, SendStrandRequest, Soul, Strand,
    StrandBudgetSnapshot, StrandDetail, StrandMaterial, StrandRuntimeSnapshot, WebhookSubscription,
};
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};

use super::{ApiError, ingress::ingest_webhook, sse::strand_events};

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
        .route("/api/v1/strands/{strand_id}/fork", post(fork_strand))
        .route("/api/v1/strands/{strand_id}/compact", post(compact_exec))
        .route("/api/v1/strands/{strand_id}/budget", get(strand_budget))
        .route(
            "/api/v1/strands/{strand_id}/rejections",
            get(strand_rejections),
        )
        .route("/api/v1/compacts/{compact_id}", get(compact_query))
        .route("/api/v1/strands/{strand_id}/runtime", get(runtime_snapshot))
        // IM layer (orthogonal to the runtime; shares the server for cold-start):
        // send into a soul's IM conversation, poll a participant's passive inbox.
        .route("/api/v1/im/send", post(send_im))
        .route("/api/v1/im/inbox/{participant_id}", get(poll_im))
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
    responses((status = 200, body = HealthResponse))
)]
pub(super) async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        ok: true,
        service: "santi-api".to_string(),
    })
}

#[utoipa::path(
    post,
    path = "/api/v1/strands",
    responses((status = 200, body = CreateStrandResponse), (status = 500, body = ErrorResponse))
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
    responses((status = 200, body = [Strand]), (status = 500, body = ErrorResponse))
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
    responses((status = 200, body = Soul), (status = 500, body = ErrorResponse))
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
    responses((status = 200, body = [Soul]), (status = 500, body = ErrorResponse))
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
        (status = 404, body = ErrorResponse),
        (status = 500, body = ErrorResponse)
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
    responses((status = 200, body = WebhookSubscription), (status = 500, body = ErrorResponse))
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
    responses((status = 200, body = [WebhookSubscription]), (status = 500, body = ErrorResponse))
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
        (status = 404, body = ErrorResponse),
        (status = 500, body = ErrorResponse)
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
        (status = 404, body = ErrorResponse),
        (status = 500, body = ErrorResponse)
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
        (status = 404, body = ErrorResponse),
        (status = 500, body = ErrorResponse)
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
        (status = 404, body = ErrorResponse),
        (status = 500, body = ErrorResponse)
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
        .map_err(ApiError::from_service)
}

#[utoipa::path(
    post,
    path = "/api/v1/strands/{strand_id}/fork",
    params(("strand_id" = String, Path)),
    responses(
        (status = 200, body = ForkStrandResponse),
        (status = 404, body = ErrorResponse),
        (status = 500, body = ErrorResponse)
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
        (status = 400, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
        (status = 500, body = ErrorResponse)
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
        (status = 404, body = ErrorResponse),
        (status = 500, body = ErrorResponse)
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
        (status = 404, body = ErrorResponse),
        (status = 500, body = ErrorResponse)
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
        (status = 404, body = ErrorResponse),
        (status = 500, body = ErrorResponse)
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

#[utoipa::path(
    get,
    path = "/api/v1/strands/{strand_id}/rejections",
    params(
        ("strand_id" = String, Path),
        ("limit" = Option<i64>, Query)
    ),
    responses(
        (status = 200, body = Vec<RejectedDelivery>),
        (status = 404, body = ErrorResponse),
        (status = 500, body = ErrorResponse)
    )
)]
pub(super) async fn strand_rejections(
    State(service): State<SantiService>,
    Path(strand_id): Path<String>,
    Query(params): Query<RejectionQueryParams>,
) -> Result<Json<Vec<RejectedDelivery>>, ApiError> {
    service
        .strand_rejections(&strand_id, params.limit.unwrap_or(50))
        .map_err(ApiError::from_service)?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("strand not found"))
}

#[derive(serde::Deserialize)]
pub(super) struct RejectionQueryParams {
    limit: Option<i64>,
}

// ── IM layer routes ─────────────────────────────────────────────────────────
// The plain IM integrated into santi. `strand send`/the runtime stay source-less;
// the participant address is IM envelope only. Inbound reuses the runtime primitive
// (Text into an `im:<participant>` conversation strand). The reply comes back into
// the participant's passive inbox (written by the soul's offline `im reply` egress).

#[utoipa::path(
    post,
    path = "/api/v1/im/send",
    request_body = ImSendRequest,
    responses(
        (status = 200, body = ImSendResponse),
        (status = 404, body = ErrorResponse),
        (status = 500, body = ErrorResponse)
    )
)]
pub(super) async fn send_im(
    State(service): State<SantiService>,
    Json(request): Json<ImSendRequest>,
) -> Result<Json<ImSendResponse>, ApiError> {
    let outcome = service
        .im_send(&request.soul_id, &request.participant_id, &request.content)
        .map_err(ApiError::from_service)?;
    let response = match outcome {
        IngestOutcome::Accepted { strand_id } => ImSendResponse {
            accepted: true,
            participant_id: request.participant_id,
            strand_id: Some(strand_id),
            reason: None,
        },
        IngestOutcome::Rejected { reason } => ImSendResponse {
            accepted: false,
            participant_id: request.participant_id,
            strand_id: None,
            reason: Some(reason),
        },
    };
    Ok(Json(response))
}

#[utoipa::path(
    get,
    path = "/api/v1/im/inbox/{participant_id}",
    params(
        ("participant_id" = String, Path),
        ("since" = Option<i64>, Query)
    ),
    responses(
        (status = 200, body = Vec<ImInboxEntry>),
        (status = 500, body = ErrorResponse)
    )
)]
pub(super) async fn poll_im(
    State(service): State<SantiService>,
    Path(participant_id): Path<String>,
    Query(params): Query<ImPollParams>,
) -> Result<Json<Vec<ImInboxEntry>>, ApiError> {
    service
        .im_poll(&participant_id, params.since.unwrap_or(0))
        .map(Json)
        .map_err(ApiError::from_service)
}

#[derive(serde::Deserialize)]
pub(super) struct ImPollParams {
    since: Option<i64>,
}

pub(super) async fn openapi() -> Json<utoipa::openapi::OpenApi> {
    Json(super::openapi::document())
}
