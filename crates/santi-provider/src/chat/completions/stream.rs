use async_stream::try_stream;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{ProviderEvent, ProviderFunctionCall, ProviderStreamTrace};

use super::*;

pub(super) fn parse_sse(
    mut bytes: impl Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Unpin + Send + 'static,
) -> impl Stream<Item = Result<ProviderEvent, String>> + Send + 'static {
    try_stream! {
        let mut buffer = String::new();
        let mut accumulator = ToolCallAccumulator::default();
        let mut response_id: Option<String> = None;
        while let Some(chunk) = bytes.next().await {
            let chunk = chunk.map_err(|error| error.to_string())?;
            yield ProviderEvent::StreamTrace(ProviderStreamTrace::Chunk { bytes: chunk.len() });
            buffer.push_str(&String::from_utf8_lossy(&chunk));
            for event in parse_buffer(&mut buffer, &mut response_id, &mut accumulator)? {
                yield event;
            }
        }
    }
}

fn parse_buffer(
    buffer: &mut String,
    response_id: &mut Option<String>,
    accumulator: &mut ToolCallAccumulator,
) -> Result<Vec<ProviderEvent>, String> {
    let mut mapped = Vec::new();
    for line in crate::sse::lines(buffer) {
        let Some(payload) = crate::sse::data(&line) else {
            continue;
        };
        let events = parse_event(payload, response_id, accumulator)?;
        mapped.push(ProviderEvent::StreamTrace(ProviderStreamTrace::RawEvent {
            raw_type: raw_event_type(payload),
            mapped_events: provider_event_names(&events),
        }));
        mapped.extend(events);
    }
    Ok(mapped)
}

fn parse_event(
    payload: &str,
    response_id: &mut Option<String>,
    accumulator: &mut ToolCallAccumulator,
) -> Result<Vec<ProviderEvent>, String> {
    let chunk = serde_json::from_str::<ChatChunk>(payload).map_err(|error| error.to_string())?;
    let mut events = Vec::new();
    if response_id.is_none() {
        *response_id = Some(chunk.id.clone());
        events.push(ProviderEvent::ResponseStarted {
            provider_response_id: response_id.clone(),
        });
    }
    for choice in chunk.choices {
        let delta = choice.delta;
        if let Some(reasoning) = delta.reasoning_content.filter(|value| !value.is_empty()) {
            events.push(ProviderEvent::ReasoningSummaryDelta(reasoning));
        }
        if let Some(content) = delta.content.filter(|value| !value.is_empty()) {
            events.push(ProviderEvent::TextDelta(content));
        }
        if let Some(tool_calls) = delta.tool_calls {
            accumulator.push(tool_calls);
        }
        let finish_reason = choice.finish_reason.as_deref();
        if finish_reason == Some("tool_calls") {
            events.extend(accumulator.finish(response_id.clone())?);
        } else if finish_reason == Some("stop") || finish_reason == Some("length") {
            events.push(ProviderEvent::Completed {
                provider_response_id: response_id.clone(),
            });
        }
    }
    Ok(events)
}

fn raw_event_type(payload: &str) -> String {
    serde_json::from_str::<ChatChunk>(payload)
        .map(|chunk| {
            chunk
                .choices
                .first()
                .and_then(|choice| choice.finish_reason.clone())
                .map(|finish_reason| format!("chat.completion.chunk.{finish_reason}"))
                .unwrap_or_else(|| "chat.completion.chunk".to_string())
        })
        .unwrap_or_else(|_| "invalid_json".to_string())
}

fn provider_event_names(events: &[ProviderEvent]) -> Vec<String> {
    events
        .iter()
        .map(provider_event_name)
        .map(str::to_string)
        .collect()
}

fn provider_event_name(event: &ProviderEvent) -> &'static str {
    match event {
        ProviderEvent::StreamTrace(_) => "stream_trace",
        ProviderEvent::ResponseStarted { .. } => "response_started",
        ProviderEvent::ResponseInProgress { .. } => "response_in_progress",
        ProviderEvent::ReasoningSummaryDelta(_) => "reasoning_summary_delta",
        ProviderEvent::ReasoningSummaryDone(_) => "reasoning_summary_done",
        ProviderEvent::TextDelta(_) => "text_delta",
        ProviderEvent::FunctionCallRequested(_) => "function_call_requested",
        ProviderEvent::Completed { .. } => "completed",
        ProviderEvent::Failed(_) => "failed",
    }
}

#[derive(Debug, Default)]
struct ToolCallAccumulator {
    calls: Vec<AccumulatedToolCall>,
}

impl ToolCallAccumulator {
    fn push(&mut self, tool_calls: Vec<ChatToolCallDelta>) {
        for tool_call in tool_calls {
            let index = tool_call.index;
            while self.calls.len() <= index {
                self.calls.push(AccumulatedToolCall::default());
            }
            let target = &mut self.calls[index];
            if let Some(id) = tool_call.id {
                target.id = id;
            }
            if let Some(function) = tool_call.function {
                if let Some(name) = function.name.filter(|name| !name.is_empty()) {
                    target.name = name;
                }
                if let Some(arguments) = function.arguments {
                    target.arguments.push_str(&arguments);
                }
            }
        }
    }

    fn finish(&mut self, response_id: Option<String>) -> Result<Vec<ProviderEvent>, String> {
        let response_id =
            response_id.ok_or_else(|| "missing chat completions response id".to_string())?;
        let calls = std::mem::take(&mut self.calls);
        calls
            .into_iter()
            .map(|call| call.into_provider_event(&response_id))
            .collect()
    }
}

#[derive(Debug, Default)]
struct AccumulatedToolCall {
    id: String,
    name: String,
    arguments: String,
}

impl AccumulatedToolCall {
    fn into_provider_event(self, response_id: &str) -> Result<ProviderEvent, String> {
        let arguments_raw = if self.arguments.trim().is_empty() {
            "{}".to_string()
        } else {
            self.arguments
        };
        let arguments = serde_json::from_str::<Value>(&arguments_raw)
            .map_err(|error| format!("invalid chat completions tool arguments: {error}"))?;
        Ok(ProviderEvent::FunctionCallRequested(ProviderFunctionCall {
            response_id: response_id.to_string(),
            item_id: Some(self.id.clone()),
            item: json!({
                "type": "function_call",
                "id": self.id,
                "call_id": self.id,
                "name": self.name,
                "arguments": arguments_raw,
            }),
            call_id: self.id,
            name: self.name,
            arguments_raw,
            arguments,
        }))
    }
}

#[derive(Debug, Deserialize)]
struct ChatChunk {
    id: String,
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    delta: ChatDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    reasoning_content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ChatToolCallDelta>>,
}

#[derive(Debug, Deserialize)]
struct ChatToolCallDelta {
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<ChatFunctionDelta>,
}

#[derive(Debug, Deserialize)]
struct ChatFunctionDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}
