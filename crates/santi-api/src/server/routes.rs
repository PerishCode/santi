use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use santi_core::service::Service;
use santi_core::{
    CompactExecRequest, CompactExecResponse, CompactQueryResponse, CreateDownstreamRequest,
    CreateSoulRequest, CreateStrandResponse, CreateWebhookRequest, DownstreamCredential,
    DriveStrandResponse, Fault, ForkStrandResponse, HealthResponse, IngestRequest, MaterialRequest,
    ReceiptStatus, SendStrandAcceptedResponse, SendStrandRequest, Soul, Strand,
    StrandBudgetSnapshot, StrandDetail, StrandMaterial, StrandRuntimeSnapshot, TurnEventBatch,
    WebhookSubscription,
};
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};

use super::{
    ApiError,
    errors::{errors, strand_errors},
    ingress::ingest_webhook,
    sse::{error_events, strand_events, turn_event_stream},
};

pub(super) fn router(service: Service) -> Router {
    let api = Router::new()
        .route("/api/v1/openapi.json", get(openapi))
        .route("/api/v1/strands", post(create_strand).get(list_strands))
        .route("/api/v1/souls", post(create_soul).get(list_souls))
        .route("/api/v1/souls/{soul}", get(get_soul))
        .route("/api/v1/webhooks", post(create_webhook).get(list_webhooks))
        .route("/api/v1/strands/{strand}", get(get_strand))
        .route("/api/v1/strands/{strand}/messages", get(list_messages))
        .route("/api/v1/strands/{strand}/materials", post(strand_material))
        .route("/api/v1/strands/{strand}/events", get(strand_events))
        .route("/api/v1/strands/{strand}/send", post(send_strand))
        .route("/api/v1/strands/{strand}/drive", post(drive_strand))
        .route("/api/v1/strands/{strand}/fork", post(fork_strand))
        .route("/api/v1/strands/{strand}/compact", post(compact_exec))
        .route("/api/v1/strands/{strand}/budget", get(strand_budget))
        .route("/api/v1/strands/{strand}/errors", get(strand_errors))
        .route("/api/v1/errors/events", get(error_events))
        .route("/api/v1/errors/{scope_kind}/{scope_id}", get(errors))
        .route("/api/v1/receipts/{inbox}", get(receipt_status))
        .route(
            "/api/v1/effects/{effect_id}",
            get(super::effects::effect_status),
        )
        .route(
            "/api/v1/effects/{effect_id}/resolve",
            post(super::effects::resolve_effect),
        )
        .route("/api/v1/compacts/{compact}", get(compact_query))
        .route("/api/v1/strands/{strand}/runtime", get(runtime_snapshot))
        .route("/api/v1/turn-events", get(turn_events))
        .route("/api/v1/turn-events/stream", get(turn_event_stream))
        .route(
            "/api/v1/downstreams",
            post(create_downstream).get(list_downstreams),
        )
        .route("/api/v1/ingest", post(ingest))
        .route(
            "/api/v1/bucket/{soul}/{strand}/{*key}",
            get(crate::bucket::get_bucket_object),
        );

    Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/webhooks/{name}", post(ingest_webhook))
        .merge(api)
        .route("/api/{*rest}", axum::routing::any(missing))
        .layer(TraceLayer::new_for_http())
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .with_state(service)
}

async fn missing() -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({"error": "unknown api path"})),
    )
}

#[utoipa::path(
    get,
    path = "/api/v1/health",
    responses(
        (status = 200, body = HealthResponse),
        (status = 503, body = HealthResponse)
    )
)]
pub async fn health(State(service): State<Service>) -> impl IntoResponse {
    let incidents = service.active_drive_incident_count();
    let degraded = service.is_drive_degraded() || incidents > 0;
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
            incidents,
        }),
    )
}

mod downstream;
mod drive;
mod receipts;
mod turn;
pub use downstream::*;
pub use drive::*;
pub use receipts::*;
pub use turn::*;
