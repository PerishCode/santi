use async_stream::try_stream;
use futures_util::StreamExt;
use serde_json::Value;

use crate::{Call, Event, Trace};

use super::*;

pub(super) fn frames(
    mut bytes: impl Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Unpin + Send + 'static,
) -> impl Stream<Item = Result<Event, String>> + Send + 'static {
    try_stream! {
        let mut buffer = String::new();
        let mut current: Option<String> = None;
        while let Some(chunk) = bytes.next().await {
            let chunk = chunk.map_err(|error| error.to_string())?;
            yield Event::Traced(Trace::Chunk { bytes: chunk.len() });
            buffer.push_str(&String::from_utf8_lossy(&chunk));
            for event in drained(&mut buffer, &mut current)? {
                yield event;
            }
        }
    }
}

fn drained(buffer: &mut String, response: &mut Option<String>) -> Result<Vec<Event>, String> {
    let mut mapped = Vec::new();
    for line in crate::sse::lines(buffer) {
        let Some(payload) = crate::sse::data(&line) else {
            continue;
        };
        let events = framed(payload, response)?;
        mapped.push(Event::Traced(Trace::Raw {
            kind: kind(payload),
            mapped: named(&events),
        }));
        mapped.extend(events);
    }
    Ok(mapped)
}

fn kind(payload: &str) -> String {
    serde_json::from_str::<Kind>(payload)
        .map(|event| event.kind)
        .unwrap_or_else(|_| "invalid_json".to_string())
}

fn named(events: &[Event]) -> Vec<String> {
    events.iter().map(name).map(str::to_string).collect()
}

fn name(event: &Event) -> &'static str {
    match event {
        Event::Started { .. } => "response_started",
        Event::Working { .. } => "response_in_progress",
        Event::Thinking(_) => "reasoning_summary_delta",
        Event::Thought(_) => "reasoning_summary_done",
        Event::Text(_) => "text_delta",
        Event::Called(_) => "function_call_requested",
        Event::Completed { .. } => "completed",
        Event::Failed(_) => "failed",
        Event::Traced(_) => "stream_trace",
    }
}

fn framed(payload: &str, current: &mut Option<String>) -> Result<Vec<Event>, String> {
    let value = serde_json::from_str::<Frame>(payload).map_err(|error| error.to_string())?;
    match value.kind.as_str() {
        "response.created" => {
            if let Some(response) = value.response() {
                *current = Some(response);
            }
            Ok(vec![Event::Started {
                response: current.clone(),
            }])
        }
        "response.in_progress" => {
            if let Some(response) = value.response() {
                *current = Some(response);
            }
            Ok(vec![Event::Working {
                response: current.clone(),
            }])
        }
        "response.output_text.delta" => Ok(value
            .delta
            .filter(|delta| !delta.is_empty())
            .map(|delta| vec![Event::Text(delta)])
            .unwrap_or_default()),
        "response.output_text.done" => Ok(Vec::new()),
        "response.reasoning_summary_text.delta" | "response.reasoning_summary.delta" => Ok(value
            .delta
            .filter(|delta| !delta.is_empty())
            .map(|delta| vec![Event::Thinking(delta)])
            .unwrap_or_default()),
        "response.reasoning_summary_text.done" | "response.reasoning_summary.done" => Ok(value
            .text()
            .filter(|text| !text.is_empty())
            .map(|text| vec![Event::Thought(text)])
            .unwrap_or_default()),
        "response.output_item.done" => finished(value.raw, current),
        "response.completed" => Ok(vec![Event::Completed {
            response: value.response(),
        }]),
        "error" => Ok(vec![Event::Failed(
            value
                .error
                .and_then(|error| error.message)
                .unwrap_or_else(|| "openai stream error".to_string()),
        )]),
        _ => Ok(Vec::new()),
    }
}

fn finished(raw: Value, current: &Option<String>) -> Result<Vec<Event>, String> {
    let Some(item) = raw.get("item") else {
        return Ok(Vec::new());
    };
    match item.get("type").and_then(Value::as_str) {
        Some("function_call") => parsed(item, &raw, current),
        Some("reasoning") => Ok(summarized(item).map(Event::Thought).into_iter().collect()),
        _ => Ok(Vec::new()),
    }
}

fn parsed(item: &Value, raw: &Value, current: &Option<String>) -> Result<Vec<Event>, String> {
    let response = current
        .clone()
        .or_else(|| dug(raw))
        .ok_or_else(|| "missing response id for function call".to_string())?;
    let call = item
        .get("call_id")
        .and_then(Value::as_str)
        .ok_or_else(|| "missing function_call call_id".to_string())?
        .to_string();
    let name = item
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| "missing function_call name".to_string())?
        .to_string();
    let raw = item
        .get("arguments")
        .and_then(Value::as_str)
        .unwrap_or("{}")
        .to_string();
    let arguments = serde_json::from_str::<Value>(&raw)
        .map_err(|error| format!("invalid function_call arguments: {error}"))?;

    Ok(vec![Event::Called(Call {
        response,
        mark: item.get("id").and_then(Value::as_str).map(str::to_string),
        item: item.clone(),
        call,
        name,
        raw,
        arguments,
    })])
}

fn summarized(item: &Value) -> Option<String> {
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

fn dug(value: &Value) -> Option<String> {
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
struct Frame {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    delta: Option<String>,
    #[serde(default)]
    response: Option<Reply>,
    #[serde(default, rename = "response_id")]
    held: Option<String>,
    #[serde(default)]
    error: Option<Trouble>,
    #[serde(flatten)]
    raw: Value,
}

#[derive(Debug, Deserialize)]
struct Kind {
    #[serde(rename = "type")]
    kind: String,
}

impl Frame {
    fn response(&self) -> Option<String> {
        self.response
            .as_ref()
            .and_then(|response| response.id.clone())
            .or_else(|| self.held.clone())
            .or_else(|| dug(&self.raw))
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
struct Reply {
    id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Trouble {
    message: Option<String>,
    #[allow(dead_code)]
    raw: Option<Value>,
}
