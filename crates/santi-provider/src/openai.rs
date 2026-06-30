use async_stream::try_stream;
use async_trait::async_trait;
use futures_core::Stream;
use futures_util::StreamExt;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::sync::Arc;

use crate::{
    ProviderClient, ProviderEvent, ProviderFunctionCall, ProviderItem, ProviderMetadata,
    ProviderRequest, ProviderStream, ProviderStreamTrace, ProviderTool,
};

#[derive(Debug, Clone)]
pub struct OpenAIProviderConfig {
    pub api_key: String,
    pub model: String,
    pub base_url: String,
    pub reasoning_effort: Option<String>,
    pub reasoning_summary: Option<String>,
    pub max_output_tokens: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct OpenAIProvider {
    config: OpenAIProviderConfig,
    client: Client,
}

impl OpenAIProvider {
    pub fn new(config: OpenAIProviderConfig) -> Self {
        Self {
            config,
            client: Client::new(),
        }
    }
}

#[async_trait]
impl ProviderClient for OpenAIProvider {
    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata {
            provider: Arc::from("openai"),
            model: self.config.model.clone(),
        }
    }

    async fn stream_response(&self, request: ProviderRequest) -> Result<ProviderStream, String> {
        let response = self
            .client
            .post(format!(
                "{}/responses",
                self.config.base_url.trim_end_matches('/')
            ))
            .bearer_auth(&self.config.api_key)
            .json(&response_body(&self.config, request))
            .send()
            .await
            .map_err(|error| error.to_string())?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("openai responses request failed: {status} {body}"));
        }
        Ok(Box::pin(parse_sse(response.bytes_stream())))
    }
}

fn response_body(config: &OpenAIProviderConfig, request: ProviderRequest) -> Value {
    let mut body = Map::from_iter([
        ("model".to_string(), json!(request.model)),
        ("input".to_string(), response_input(&request)),
        ("stream".to_string(), json!(true)),
        ("store".to_string(), json!(false)),
        (
            "stream_options".to_string(),
            json!({
                "include_obfuscation": false
            }),
        ),
    ]);

    if let Some(instructions) = request
        .instructions
        .filter(|instructions| !instructions.trim().is_empty())
    {
        body.insert("instructions".to_string(), json!(instructions));
    }
    if let Some(tools) = request.tools {
        body.insert("tools".to_string(), json!(map_tools(tools)));
    }
    if let Some(previous_response_id) = request.previous_response_id {
        body.insert(
            "previous_response_id".to_string(),
            json!(previous_response_id),
        );
    }
    if let Some(reasoning) = reasoning_options(config) {
        body.insert("reasoning".to_string(), reasoning);
    }
    if let Some(max_output_tokens) = config.max_output_tokens {
        body.insert("max_output_tokens".to_string(), json!(max_output_tokens));
    }

    Value::Object(body)
}

fn reasoning_options(config: &OpenAIProviderConfig) -> Option<Value> {
    let mut reasoning = Map::new();
    if let Some(effort) = config
        .reasoning_effort
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        reasoning.insert("effort".to_string(), json!(effort));
    }
    if let Some(summary) = config
        .reasoning_summary
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        reasoning.insert("summary".to_string(), json!(summary));
    }
    if reasoning.is_empty() {
        None
    } else {
        Some(Value::Object(reasoning))
    }
}

fn response_input(request: &ProviderRequest) -> Value {
    let mut items = Vec::new();
    for item in &request.input {
        match item {
            ProviderItem::Message { role, content } => {
                let content_type = if role == "assistant" {
                    "output_text"
                } else {
                    "input_text"
                };
                items.push(json!({
                    "role": role,
                    "content": [
                        {
                            "type": content_type,
                            "text": content,
                        }
                    ],
                }));
            }
            // Reasoning replay needs the encrypted reasoning item, not modeled
            // yet (DC1a) — skip until a Responses reasoning model is live.
            ProviderItem::Reasoning { .. } => {}
            ProviderItem::FunctionCall {
                call_id,
                name,
                arguments_raw,
                item,
                ..
            } => {
                // Replay the provider's raw item verbatim when present; else
                // synthesize a function_call item from stored fields.
                items.push(item.clone().unwrap_or_else(|| {
                    json!({
                        "type": "function_call",
                        "call_id": call_id,
                        "name": name,
                        "arguments": arguments_raw,
                    })
                }));
            }
            ProviderItem::FunctionCallOutput { call_id, output } => {
                items.push(json!({
                    "type": "function_call_output",
                    "call_id": call_id,
                    "output": output,
                }));
            }
        }
    }
    json!(items)
}

fn map_tools(tools: Vec<ProviderTool>) -> Vec<Value> {
    tools
        .into_iter()
        .map(|tool| match tool {
            ProviderTool::Function(tool) => json!({
                "type": "function",
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.parameters,
            }),
        })
        .collect()
}

fn parse_sse(
    mut bytes: impl Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Unpin + Send + 'static,
) -> impl Stream<Item = Result<ProviderEvent, String>> + Send + 'static {
    try_stream! {
        let mut buffer = String::new();
        let mut current_response_id: Option<String> = None;
        while let Some(chunk) = bytes.next().await {
            let chunk = chunk.map_err(|error| error.to_string())?;
            yield ProviderEvent::StreamTrace(ProviderStreamTrace::Chunk { bytes: chunk.len() });
            buffer.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(index) = buffer.find('\n') {
                let line = buffer[..index].trim_end_matches('\r').to_string();
                buffer = buffer[index + 1..].to_string();
                if let Some(payload) = line.strip_prefix("data: ") {
                    if payload == "[DONE]" {
                        continue;
                    }
                    let events = parse_event(payload, &mut current_response_id)?;
                    yield ProviderEvent::StreamTrace(ProviderStreamTrace::RawEvent {
                        raw_type: raw_event_type(payload),
                        mapped_events: provider_event_names(&events),
                    });
                    for event in events {
                        yield event;
                    }
                }
            }
        }
    }
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
