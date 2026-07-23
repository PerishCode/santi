use async_stream::try_stream;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{Call, Event, Trace};

use super::*;

pub(super) fn frames(
    mut bytes: impl Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Unpin + Send + 'static,
) -> impl Stream<Item = Result<Event, String>> + Send + 'static {
    try_stream! {
        let mut buffer = String::new();
        let mut accumulator = Gatherer::default();
        let mut response: Option<String> = None;
        while let Some(chunk) = bytes.next().await {
            let chunk = chunk.map_err(|error| error.to_string())?;
            yield Event::Traced(Trace::Chunk { bytes: chunk.len() });
            buffer.push_str(&String::from_utf8_lossy(&chunk));
            for event in drained(&mut buffer, &mut response, &mut accumulator)? {
                yield event;
            }
        }
    }
}

fn drained(
    buffer: &mut String,
    response: &mut Option<String>,
    accumulator: &mut Gatherer,
) -> Result<Vec<Event>, String> {
    let mut mapped = Vec::new();
    for line in crate::sse::lines(buffer) {
        let Some(payload) = crate::sse::data(&line) else {
            continue;
        };
        let events = framed(payload, response, accumulator)?;
        mapped.push(Event::Traced(Trace::Raw {
            kind: kind(payload),
            mapped: named(&events),
        }));
        mapped.extend(events);
    }
    Ok(mapped)
}

fn framed(
    payload: &str,
    response: &mut Option<String>,
    accumulator: &mut Gatherer,
) -> Result<Vec<Event>, String> {
    let chunk = serde_json::from_str::<Chunk>(payload).map_err(|error| error.to_string())?;
    let mut events = Vec::new();
    if response.is_none() {
        *response = Some(chunk.id.clone());
        events.push(Event::Started {
            response: response.clone(),
        });
    }
    for choice in chunk.choices {
        let delta = choice.delta;
        if let Some(reasoning) = delta.reasoning.filter(|value| !value.is_empty()) {
            events.push(Event::Thinking(reasoning));
        }
        if let Some(content) = delta.content.filter(|value| !value.is_empty()) {
            events.push(Event::Text(content));
        }
        if let Some(calls) = delta.calls {
            accumulator.push(calls);
        }
        let finish = choice.finish.as_deref();
        if finish == Some("tool_calls") {
            events.extend(accumulator.finish(response.clone())?);
        } else if finish == Some("stop") || finish == Some("length") {
            events.push(Event::Completed {
                response: response.clone(),
            });
        }
    }
    Ok(events)
}

fn kind(payload: &str) -> String {
    serde_json::from_str::<Chunk>(payload)
        .map(|chunk| {
            chunk
                .choices
                .first()
                .and_then(|choice| choice.finish.clone())
                .map(|finish| format!("chat.completion.chunk.{finish}"))
                .unwrap_or_else(|| "chat.completion.chunk".to_string())
        })
        .unwrap_or_else(|_| "invalid_json".to_string())
}

fn named(events: &[Event]) -> Vec<String> {
    events.iter().map(name).map(str::to_string).collect()
}

fn name(event: &Event) -> &'static str {
    match event {
        Event::Traced(_) => "stream_trace",
        Event::Started { .. } => "response_started",
        Event::Working { .. } => "response_in_progress",
        Event::Thinking(_) => "reasoning_summary_delta",
        Event::Thought(_) => "reasoning_summary_done",
        Event::Text(_) => "text_delta",
        Event::Called(_) => "function_call_requested",
        Event::Completed { .. } => "completed",
        Event::Failed(_) => "failed",
    }
}

#[derive(Debug, Default)]
struct Gatherer {
    calls: Vec<Gathered>,
}

impl Gatherer {
    fn push(&mut self, calls: Vec<Delta>) {
        for tool_call in calls {
            let index = tool_call.index;
            while self.calls.len() <= index {
                self.calls.push(Gathered::default());
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

    fn finish(&mut self, response: Option<String>) -> Result<Vec<Event>, String> {
        let response =
            response.ok_or_else(|| "missing chat completions response id".to_string())?;
        let calls = std::mem::take(&mut self.calls);
        calls
            .into_iter()
            .map(|call| call.projected(&response))
            .collect()
    }
}

#[derive(Debug, Default)]
struct Gathered {
    id: String,
    name: String,
    arguments: String,
}

impl Gathered {
    fn projected(self, response: &str) -> Result<Event, String> {
        let raw = if self.arguments.trim().is_empty() {
            "{}".to_string()
        } else {
            self.arguments
        };
        let arguments = serde_json::from_str::<Value>(&raw)
            .map_err(|error| format!("invalid chat completions tool arguments: {error}"))?;
        Ok(Event::Called(Call {
            response: response.to_string(),
            mark: Some(self.id.clone()),
            item: json!({
                "type": "function_call",
                "id": self.id,
                "call_id": self.id,
                "name": self.name,
                "arguments": raw,
            }),
            call: self.id,
            name: self.name,
            raw,
            arguments,
        }))
    }
}

#[derive(Debug, Deserialize)]
struct Chunk {
    id: String,
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    delta: Speech,
    #[serde(default, rename = "finish_reason")]
    finish: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Speech {
    #[serde(default)]
    content: Option<String>,
    #[serde(default, rename = "reasoning_content")]
    reasoning: Option<String>,
    #[serde(default, rename = "tool_calls")]
    calls: Option<Vec<Delta>>,
}

#[derive(Debug, Deserialize)]
struct Delta {
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<Sliver>,
}

#[derive(Debug, Deserialize)]
struct Sliver {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}
