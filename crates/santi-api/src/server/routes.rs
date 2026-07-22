use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use santi_core::service::Service;
use santi_core::{
    CompactExecRequest, CompactExecResponse, CompactQueryResponse, CreateSoulRequest,
    CreateStrandResponse, CreateWebhookRequest, DriveStrandResponse, ForkStrandResponse,
    HealthResponse, MaterialRequest, ReceiptStatus, SantiError, SendStrandAcceptedResponse,
    SendStrandRequest, Soul, Strand, StrandBudgetSnapshot, StrandDetail, StrandMaterial,
    StrandRuntimeSnapshot, TurnEventPage, WebhookSubscription,
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

pub(super) fn router(service: Service) -> Router {
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
        .route("/api/v1/turn-events", get(turn_events))
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

mod drive;
mod receipts;
mod turn;
pub use drive::*;
pub use receipts::*;
pub use turn::*;
