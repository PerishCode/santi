use std::env;

use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use santi_core::Fault;
use santi_core::service::Service;
use serde_json::json;

use crate::webhook::{WebhookOutcome, adaptor_for};

use super::ApiError;
use santi_core::ingest;

#[utoipa::path(
    post,
    path = "/api/v1/webhooks/{name}",
    summary = "Webhook ingest endpoint. Not bearer-gated — authenticity is established by the\nadaptor verifying the request signature against the subscription's secret. An\nout-of-scope or self-authored event returns 200 without waking the soul.",
    params(("name" = String, Path)),
    request_body(content_type = "application/json", description = "Raw provider event payload"),
    responses(
        (status = 200, description = "Control response or ignored event"),
        (status = 202, body = santi_core::ingest::Receipt),
        (status = 401, body = Fault),
        (status = 423, body = Fault),
        (status = 404, body = Fault),
        (status = 500, body = Fault),
        (status = 503, body = Fault)
    )
)]
pub(super) async fn ingest_webhook(
    State(service): State<Service>,
    Path(name): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let subscription = service
        .webhook(&name)
        .map_err(ApiError::from_service)?
        .ok_or_else(|| ApiError::not_found("webhook not found"))?;
    let adaptor = adaptor_for(&subscription.adaptor)
        .ok_or_else(|| ApiError::internal(format!("unknown adaptor {}", subscription.adaptor)))?;
    let secret = env::var(&subscription.credential)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ApiError::unauthorized(format!(
                "webhook secret env {} is not set",
                subscription.credential
            ))
        })?;
    adaptor
        .verify(&headers, &body, &secret)
        .map_err(ApiError::from_webhook)?;
    let event = match adaptor
        .normalize(&headers, &body, &secret, &name)
        .map_err(ApiError::from_webhook)?
    {
        WebhookOutcome::Reply(reply) => return Ok((StatusCode::OK, Json(reply)).into_response()),
        WebhookOutcome::Event(event) => event,
    };
    if !event.in_scope || event.self_authored {
        return Ok(StatusCode::OK.into_response());
    }
    let label = if subscription.strategy == "single" {
        format!("{}:{}", subscription.adaptor, name)
    } else {
        event.label.clone()
    };
    let source = ingest::Source::new("webhook")
        .with_ref(format!("{}:{name}", subscription.adaptor))
        .with_metadata(json!({
            "subscription": name,
            "adaptor": subscription.adaptor,
            "strategy": subscription.strategy,
            "event_label": event.label,
            "materialized_label": label,
            "event": event.metadata,
        }));
    match service
        .sourced(
            &subscription.soul,
            &label,
            event.santi_system_text,
            Some(source),
        )
        .map_err(ApiError::from_service)?
    {
        ingest::Outcome::Accepted { receipt } => {
            Ok((StatusCode::ACCEPTED, Json(receipt)).into_response())
        }
        ingest::Outcome::Rejected { error } => Err(ApiError::from_santi(*error)),
    }
}
