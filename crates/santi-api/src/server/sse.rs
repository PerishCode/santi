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
use santi_core::{Transition, now, tag};
use tokio::sync::broadcast;

use super::ApiError;
use super::routes::bearer;
use santi_core::stream;

pub(super) async fn strand_events(
    State(service): State<Service>,
    Path(strand): Path<String>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let held = service
        .strand(&strand)
        .map_err(ApiError::from_service)?
        .ok_or_else(|| ApiError::not_found("strand not found"))?;
    drop(held);

    let mut receiver = service.listen();
    let opened = strand.clone();
    let stream = async_stream::stream! {
        yield Ok(sse_event(santi_core::stream::Event {
            id: tag("stream"),
            strand: opened,
            created: now(),
            payload: stream::Payload::StreamOpen,
        }));

        while let Some(event) = receive_strand(&mut receiver, &strand).await {
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
pub(super) async fn transitions(
    State(service): State<Service>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let mut receiver = service.harken();
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
    let mut receiver = service.listen();
    let stream = async_stream::stream! {
        while receive_turn(&mut receiver, &principal.prefix).await {
            yield Ok(Event::default().event("turn_event_available").data("{}"));
        }
    };
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

async fn receive_strand(
    receiver: &mut broadcast::Receiver<stream::Event>,
    strand: &str,
) -> Option<stream::Event> {
    loop {
        let event = receive(receiver).await?;
        if event.strand == strand {
            return Some(event);
        }
    }
}

async fn receive_turn(receiver: &mut broadcast::Receiver<stream::Event>, prefix: &str) -> bool {
    loop {
        let Some(event) = receive(receiver).await else {
            return false;
        };
        if let stream::Payload::TurnCompleted {
            label: Some(label), ..
        } = event.payload
            && label.starts_with(prefix)
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

fn sse_event(event: stream::Event) -> Event {
    Event::default()
        .id(event.id.clone())
        .event(sse_event_name(&event.payload))
        .data(serde_json::to_string(&event).unwrap_or_else(|_| "{}".to_string()))
}

fn sse_event_name(payload: &stream::Payload) -> &'static str {
    match payload {
        stream::Payload::StreamOpen => "stream_open",
        stream::Payload::MessageCreated { .. } => "message_created",
        stream::Payload::MessageDelta { .. } => "message_delta",
        stream::Payload::MessageCompleted { .. } => "message_completed",
        stream::Payload::ToolCallCreated { .. } => "tool_call_created",
        stream::Payload::ToolResultCreated { .. } => "tool_result_created",
        stream::Payload::ThinkingCreated { .. } => "thinking_created",
        stream::Payload::ThinkingUpdated { .. } => "thinking_updated",
        stream::Payload::ThinkingCompleted { .. } => "thinking_completed",
        stream::Payload::MaterialUpdated { .. } => "material_updated",
        stream::Payload::TurnStarted { .. } => "turn_started",
        stream::Payload::TurnActivity { .. } => "turn_activity",
        stream::Payload::TurnCompleted { .. } => "turn_completed",
        stream::Payload::TurnFailed { .. } => "turn_failed",
        stream::Payload::Transition { .. } => "error_transition",
    }
}
