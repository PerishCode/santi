use std::convert::Infallible;

use axum::{
    extract::{Path, State},
    response::{
        Sse,
        sse::{Event, KeepAlive},
    },
};
use futures_core::Stream;
use santi_core::{
    ErrorTransition, SantiService, SantiStreamEvent, SantiStreamPayload, prefixed_id, timestamp_now,
};

use super::ApiError;

pub(super) async fn strand_events(
    State(service): State<SantiService>,
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

        loop {
            match receiver.recv().await {
                Ok(event) if event.strand_id == strand_id => yield Ok(sse_event(event)),
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
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
    State(service): State<SantiService>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let mut receiver = service.subscribe_error_transitions();
    let stream = async_stream::stream! {
        loop {
            match receiver.recv().await {
                Ok(transition) => yield Ok(error_sse_event(transition)),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    };
    Sse::new(stream).keep_alive(KeepAlive::default())
}

fn error_sse_event(transition: ErrorTransition) -> Event {
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
        SantiStreamPayload::ErrorTransition { .. } => "error_transition",
    }
}
