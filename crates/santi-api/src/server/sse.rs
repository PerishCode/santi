use std::convert::Infallible;

use axum::{
    extract::{Path, State},
    http::HeaderMap,
    response::{
        Sse,
        sse::{Event, KeepAlive},
    },
};
use futures_core::Stream;
use santi_core::service::Service;
use santi_core::{SantiStreamEvent, SantiStreamPayload, Transition, prefixed_id, timestamp_now};
use tokio::sync::broadcast;

use super::ApiError;
use super::routes::bearer;

pub(super) async fn strand_events(
    State(service): State<Service>,
    Path(strand_id): Path<String>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let strand = service
        .strand(&strand_id)
        .map_err(ApiError::from_service)?
        .ok_or_else(|| ApiError::not_found("strand not found"))?;
    drop(strand);

    let mut receiver = service.subscribe_stream();
    let open_strand_id = strand_id.clone();
    let stream = async_stream::stream! {
        yield Ok(sse_event(SantiStreamEvent {
            event_id: prefixed_id("stream"),
            strand_id: open_strand_id,
            created_at: timestamp_now(),
            payload: SantiStreamPayload::StreamOpen,
        }));

        while let Some(event) = receive_strand(&mut receiver, &strand_id).await {
            yield Ok(sse_event(event));
        }
    };
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

#[utoipa::path(
    get,
    path = "/api/v1/errors/events",
    responses((status = 200, description = "Canonical global error lifecycle stream"))
)]
pub(super) async fn error_events(
    State(service): State<Service>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let mut receiver = service.subscribe_error_transitions();
    let stream = async_stream::stream! {
        while let Some(transition) = receive(&mut receiver).await {
            yield Ok(error_sse_event(transition));
        }
    };
    Sse::new(stream).keep_alive(KeepAlive::default())
}

#[utoipa::path(
    get,
    path = "/api/v1/turn-events/stream",
    security(("downstream_bearer" = [])),
    responses(
        (status = 200, description = "Zone-filtered turn event wake-up stream"),
        (status = 401, body = santi_core::Fault)
    )
)]
pub(super) async fn turn_event_stream(
    State(service): State<Service>,
    headers: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let principal = service
        .principal(bearer(&headers))
        .map_err(ApiError::from_service)?
        .ok_or_else(|| ApiError::unauthorized("invalid or missing credential"))?;
    let mut receiver = service.subscribe_stream();
    let stream = async_stream::stream! {
        while receive_turn(&mut receiver, &principal.label_prefix).await {
            yield Ok(Event::default().event("turn_event_available").data("{}"));
        }
    };
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

async fn receive_strand(
    receiver: &mut broadcast::Receiver<SantiStreamEvent>,
    strand_id: &str,
) -> Option<SantiStreamEvent> {
    loop {
        let event = receive(receiver).await?;
        if event.strand_id == strand_id {
            return Some(event);
        }
    }
}

async fn receive_turn(
    receiver: &mut broadcast::Receiver<SantiStreamEvent>,
    label_prefix: &str,
) -> bool {
    loop {
        let Some(event) = receive(receiver).await else {
            return false;
        };
        if let SantiStreamPayload::TurnCompleted {
            external_label: Some(label),
            ..
        } = event.payload
            && label.starts_with(label_prefix)
        {
            return true;
        }
    }
}

async fn receive<T: Clone>(receiver: &mut broadcast::Receiver<T>) -> Option<T> {
    loop {
        match receiver.recv().await {
            Ok(event) => return Some(event),
            Err(broadcast::error::RecvError::Lagged(_)) => {}
            Err(broadcast::error::RecvError::Closed) => return None,
        }
    }
}

fn error_sse_event(transition: Transition) -> Event {
    Event::default()
        .id(transition.id.clone())
        .event("error_transition")
        .data(serde_json::to_string(&transition).unwrap_or_else(|_| "{}".to_string()))
}

fn sse_event(event: SantiStreamEvent) -> Event {
    Event::default()
        .id(event.event_id.clone())
        .event(sse_event_name(&event.payload))
        .data(serde_json::to_string(&event).unwrap_or_else(|_| "{}".to_string()))
}

fn sse_event_name(payload: &SantiStreamPayload) -> &'static str {
    match payload {
        SantiStreamPayload::StreamOpen => "stream_open",
        SantiStreamPayload::MessageCreated { .. } => "message_created",
        SantiStreamPayload::MessageDelta { .. } => "message_delta",
        SantiStreamPayload::MessageCompleted { .. } => "message_completed",
        SantiStreamPayload::ToolCallCreated { .. } => "tool_call_created",
        SantiStreamPayload::ToolResultCreated { .. } => "tool_result_created",
        SantiStreamPayload::ThinkingCreated { .. } => "thinking_created",
        SantiStreamPayload::ThinkingUpdated { .. } => "thinking_updated",
        SantiStreamPayload::ThinkingCompleted { .. } => "thinking_completed",
        SantiStreamPayload::MaterialUpdated { .. } => "material_updated",
        SantiStreamPayload::TurnStarted { .. } => "turn_started",
        SantiStreamPayload::TurnActivity { .. } => "turn_activity",
        SantiStreamPayload::TurnCompleted { .. } => "turn_completed",
        SantiStreamPayload::TurnFailed { .. } => "turn_failed",
        SantiStreamPayload::Transition { .. } => "error_transition",
    }
}
