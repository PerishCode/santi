use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use santi_core::service::Service;
use santi_core::{
    Fault, Health, downstream::Credential, downstream::Draft, drive::Response, soul::Soul,
    strand::Strand,
};
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};

use super::{
    ApiError,
    errors::{errors, strand_errors},
    ingress::ingest_webhook,
    sse::{strand_events, transitions, turn_event_stream},
};

pub(super) fn router(service: Service) -> Router {
    let api = Router::new()
        .route("/api/v1/openapi.json", get(openapi))
        .route("/api/v1/strands", post(weave).get(strands))
        .route("/api/v1/souls", post(awaken).get(souls))
        .route("/api/v1/souls/{soul}", get(get_soul))
        .route("/api/v1/webhooks", post(subscribe).get(webhooks))
        .route("/api/v1/strands/{strand}", get(get_strand))
        .route("/api/v1/strands/{strand}/messages", get(list_messages))
        .route("/api/v1/strands/{strand}/materials", post(strand_material))
        .route("/api/v1/strands/{strand}/events", get(strand_events))
        .route("/api/v1/strands/{strand}/send", post(send))
        .route("/api/v1/strands/{strand}/drive", post(drive_strand))
        .route("/api/v1/strands/{strand}/fork", post(fork))
        .route("/api/v1/strands/{strand}/compact", post(exec))
        .route("/api/v1/strands/{strand}/budget", get(strand_budget))
        .route("/api/v1/strands/{strand}/errors", get(strand_errors))
        .route("/api/v1/errors/events", get(transitions))
        .route("/api/v1/errors/{scope_kind}/{scope_id}", get(errors))
        .route("/api/v1/receipts/{inbox}", get(receipt))
        .route("/api/v1/effects/{effect}", get(super::effects::effect))
        .route(
            "/api/v1/effects/{effect}/resolve",
            post(super::effects::settle),
        )
        .route("/api/v1/compacts/{compact}", get(page))
        .route("/api/v1/strands/{strand}/runtime", get(snapshot))
        .route("/api/v1/turn-events", get(turn_events))
        .route("/api/v1/turn-events/stream", get(turn_event_stream))
        .route("/api/v1/downstreams", post(enroll).get(downstreams))
        .route("/api/v1/ingest", post(ingest))
        .route(
            "/api/v1/bucket/{soul}/{strand}/{*key}",
            get(crate::bucket::fetch),
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
        (status = 200, body = Health),
        (status = 503, body = Health)
    )
)]
pub async fn health(State(service): State<Service>) -> impl IntoResponse {
    let incidents = service.strained();
    let degraded = service.degraded() || incidents > 0;
    let status = if degraded {
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        StatusCode::OK
    };
    (
        status,
        Json(Health {
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
