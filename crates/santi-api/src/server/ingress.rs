use std::env;

use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use santi_core::{ErrorResponse, InboxSource, IngestOutcome, SantiService};
use serde_json::json;

use crate::webhook::{WebhookOutcome, adaptor_for};

use super::ApiError;

/// Webhook ingest endpoint. Not bearer-gated — authenticity is established by the
/// adaptor verifying the request signature against the subscription's secret. An
/// out-of-scope or self-authored event returns 200 without waking the soul.
#[utoipa::path(
    post,
    path = "/api/v1/webhooks/{name}",
    params(("name" = String, Path)),
    request_body(content_type = "application/json", description = "Raw provider event payload"),
    responses(
        (status = 200, description = "Event accepted (turn may or may not be triggered)"),
        (status = 401, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
        (status = 500, body = ErrorResponse)
    )
)]
pub(super) async fn ingest_webhook(
    State(service): State<SantiService>,
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
    // Fail-closed: a missing or empty secret is never a pass.
    let secret = env::var(&subscription.secret_env)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ApiError::unauthorized(format!(
                "webhook secret env {} is not set",
                subscription.secret_env
            ))
        })?;
    adaptor
        .verify(&headers, &body, &secret)
        .map_err(ApiError::from_webhook)?;
    let event = match adaptor
        .normalize(&headers, &body, &secret, &name)
        .map_err(ApiError::from_webhook)?
    {
        // Control-plane request (e.g. feishu's url_verification challenge):
        // answered on the spot, never touches a strand.
        WebhookOutcome::Reply(reply) => return Ok((StatusCode::OK, Json(reply)).into_response()),
        WebhookOutcome::Event(event) => event,
    };
    // Out-of-scope events and the soul's own actions verify fine but produce no
    // turn — the loop guard and the scope filter live in the adaptor.
    if !event.in_scope || event.self_authored {
        return Ok(StatusCode::OK.into_response());
    }
    // `per_thread` anchors on the adaptor's fine-grained label; `single` collapses
    // every event for this subscription into one strand.
    let label = if subscription.strand_strategy == "single" {
        format!("{}:{}", subscription.adaptor, name)
    } else {
        event.label.clone()
    };
    let source = InboxSource::new("webhook")
        .with_ref(format!("{}:{name}", subscription.adaptor))
        .with_metadata(json!({
            "subscription": name,
            "adaptor": subscription.adaptor,
            "strand_strategy": subscription.strand_strategy,
            "event_label": event.label,
            "materialized_label": label,
            "event": event.source_metadata,
        }));
    // Rejection handling is the adaptor's own policy: a webhook silently drops
    // + logs (the sender has no way to retry a specific event) rather than
    // surfacing the inbox gate as an error.
    match service
        .ingest_external_source(
            &subscription.soul_id,
            &label,
            event.santi_system_text,
            Some(source),
        )
        .map_err(ApiError::from_service)?
    {
        IngestOutcome::Accepted { .. } => {}
        IngestOutcome::Rejected { reason } => {
            eprintln!("santi: webhook ingest rejected for {name}: {reason}");
        }
    }
    Ok(StatusCode::OK.into_response())
}
