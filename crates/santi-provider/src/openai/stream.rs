use async_stream::try_stream;
use futures_util::StreamExt;
use serde_json::Value;

use crate::{ProviderEvent, ProviderFunctionCall, ProviderStreamTrace};

use super::*;

pub(super) fn parse_sse(
    mut bytes: impl Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Unpin + Send + 'static,
) -> impl Stream<Item = Result<ProviderEvent, String>> + Send + 'static {
    try_stream! {
        let mut buffer = String::new();
        let mut current_response_id: Option<String> = None;
        while let Some(chunk) = bytes.next().await {
            let chunk = chunk.map_err(|error| error.to_string())?;
            yield ProviderEvent::StreamTrace(ProviderStreamTrace::Chunk { bytes: chunk.len() });
            buffer.push_str(&String::from_utf8_lossy(&chunk));
            for event in parse_buffer(&mut buffer, &mut current_response_id)? {
                yield event;
            }
        }
    }
}

fn parse_buffer(
    buffer: &mut String,
    response_id: &mut Option<String>,
) -> Result<Vec<ProviderEvent>, String> {
    let mut mapped = Vec::new();
    for line in crate::sse::lines(buffer) {
        let Some(payload) = crate::sse::data(&line) else {
            continue;
        };
        let events = parse_event(payload, response_id)?;
        mapped.push(ProviderEvent::StreamTrace(ProviderStreamTrace::RawEvent {
            raw_type: raw_event_type(payload),
            mapped_events: provider_event_names(&events),
        }));
        mapped.extend(events);
    }
    Ok(mapped)
}

fn raw_event_type(payload: &str) -> String {
    serde_json::from_str::<OpenAIEventKind>(payload)
        .map(|event| event.event_type)
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
        ProviderEvent::ResponseStarted { .. } => "response_started",
        ProviderEvent::ResponseInProgress { .. } => "response_in_progress",
        ProviderEvent::ReasoningSummaryDelta(_) => "reasoning_summary_delta",
        ProviderEvent::ReasoningSummaryDone(_) => "reasoning_summary_done",
        ProviderEvent::TextDelta(_) => "text_delta",
        ProviderEvent::FunctionCallRequested(_) => "function_call_requested",
        ProviderEvent::Completed { .. } => "completed",
        ProviderEvent::Failed(_) => "failed",
        ProviderEvent::StreamTrace(_) => "stream_trace",
    }
}

fn parse_event(
    payload: &str,
    current_response_id: &mut Option<String>,
) -> Result<Vec<ProviderEvent>, String> {
    let value = serde_json::from_str::<OpenAIEvent>(payload).map_err(|error| error.to_string())?;
    match value.event_type.as_str() {
        "response.created" => {
            if let Some(response_id) = value.response_id() {
                *current_response_id = Some(response_id);
            }
            Ok(vec![ProviderEvent::ResponseStarted {
                provider_response_id: current_response_id.clone(),
            }])
        }
        "response.in_progress" => {
            if let Some(response_id) = value.response_id() {
                *current_response_id = Some(response_id);
            }
            Ok(vec![ProviderEvent::ResponseInProgress {
                provider_response_id: current_response_id.clone(),
            }])
        }
        "response.output_text.delta" => Ok(value
            .delta
            .filter(|delta| !delta.is_empty())
            .map(|delta| vec![ProviderEvent::TextDelta(delta)])
            .unwrap_or_default()),
        "response.output_text.done" => Ok(Vec::new()),
        "response.reasoning_summary_text.delta" | "response.reasoning_summary.delta" => Ok(value
            .delta
            .filter(|delta| !delta.is_empty())
            .map(|delta| vec![ProviderEvent::ReasoningSummaryDelta(delta)])
            .unwrap_or_default()),
        "response.reasoning_summary_text.done" | "response.reasoning_summary.done" => Ok(value
            .text()
            .filter(|text| !text.is_empty())
            .map(|text| vec![ProviderEvent::ReasoningSummaryDone(text)])
            .unwrap_or_default()),
        "response.output_item.done" => parse_output_item_done(value.raw, current_response_id),
        "response.completed" => Ok(vec![ProviderEvent::Completed {
            provider_response_id: value.response_id(),
        }]),
        "error" => Ok(vec![ProviderEvent::Failed(
            value
                .error
                .and_then(|error| error.message)
                .unwrap_or_else(|| "openai stream error".to_string()),
        )]),
        _ => Ok(Vec::new()),
    }
}

fn parse_output_item_done(
    raw: Value,
    current_response_id: &Option<String>,
) -> Result<Vec<ProviderEvent>, String> {
    let Some(item) = raw.get("item") else {
        return Ok(Vec::new());
    };
    match item.get("type").and_then(Value::as_str) {
        Some("function_call") => parse_function_call_item(item, &raw, current_response_id),
        Some("reasoning") => Ok(reasoning_summary_from_item(item)
            .map(ProviderEvent::ReasoningSummaryDone)
            .into_iter()
            .collect()),
        _ => Ok(Vec::new()),
    }
}

fn parse_function_call_item(
    item: &Value,
    raw: &Value,
    current_response_id: &Option<String>,
) -> Result<Vec<ProviderEvent>, String> {
    let response_id = current_response_id
        .clone()
        .or_else(|| response_id_from_value(raw))
        .ok_or_else(|| "missing response id for function call".to_string())?;
    let call_id = item
        .get("call_id")
        .and_then(Value::as_str)
        .ok_or_else(|| "missing function_call call_id".to_string())?
        .to_string();
    let name = item
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| "missing function_call name".to_string())?
        .to_string();
    let arguments_raw = item
        .get("arguments")
        .and_then(Value::as_str)
        .unwrap_or("{}")
        .to_string();
    let arguments = serde_json::from_str::<Value>(&arguments_raw)
        .map_err(|error| format!("invalid function_call arguments: {error}"))?;

    Ok(vec![ProviderEvent::FunctionCallRequested(
        ProviderFunctionCall {
            response_id,
            item_id: item.get("id").and_then(Value::as_str).map(str::to_string),
            item: item.clone(),
            call_id,
            name,
            arguments_raw,
            arguments,
        },
    )])
}

fn reasoning_summary_from_item(item: &Value) -> Option<String> {
    let text =
        item.get("summary")?
            .as_array()?
            .iter()
            .fold(String::new(), |mut acc, summary_part| {
                if let Some(text) = summary_part.get("text").and_then(Value::as_str) {
                    acc.push_str(text);
                }
                acc
            });
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

fn response_id_from_value(value: &Value) -> Option<String> {
    value
        .get("response")
        .and_then(|response| response.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            value
                .get("response_id")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
}

#[derive(Debug, Deserialize)]
struct OpenAIEvent {
    #[serde(rename = "type")]
    event_type: String,
    #[serde(default)]
    delta: Option<String>,
    #[serde(default)]
    response: Option<OpenAIResponse>,
    #[serde(default)]
    response_id: Option<String>,
    #[serde(default)]
    error: Option<OpenAIError>,
    #[serde(flatten)]
    raw: Value,
}

#[derive(Debug, Deserialize)]
struct OpenAIEventKind {
    #[serde(rename = "type")]
    event_type: String,
}

impl OpenAIEvent {
    fn response_id(&self) -> Option<String> {
        self.response
            .as_ref()
            .and_then(|response| response.id.clone())
            .or_else(|| self.response_id.clone())
            .or_else(|| response_id_from_value(&self.raw))
    }

    fn text(&self) -> Option<String> {
        self.raw
            .get("text")
            .and_then(Value::as_str)
            .or_else(|| self.raw.get("summary").and_then(Value::as_str))
            .map(str::to_string)
    }
}

#[derive(Debug, Deserialize)]
struct OpenAIResponse {
    id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAIError {
    message: Option<String>,
    #[allow(dead_code)]
    raw: Option<Value>,
}
