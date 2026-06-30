use async_stream::try_stream;
use async_trait::async_trait;
use futures_core::Stream;
use futures_util::StreamExt;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::sync::Arc;

use crate::{
    FunctionCallOutput, ProviderClient, ProviderEvent, ProviderFunctionCall, ProviderMessage,
    ProviderMetadata, ProviderRequest, ProviderStream, ProviderStreamTrace, ProviderTool,
};

#[derive(Debug, Clone)]
pub struct ChatCompletionsProviderConfig {
    pub provider: String,
    pub api_key: String,
    pub model: String,
    pub base_url: String,
    pub thinking: Option<String>,
    pub reasoning_effort: Option<String>,
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct ChatCompletionsProvider {
    config: ChatCompletionsProviderConfig,
    client: Client,
}

impl ChatCompletionsProvider {
    pub fn new(config: ChatCompletionsProviderConfig) -> Self {
        Self {
            config,
            client: Client::new(),
        }
    }
}

#[async_trait]
impl ProviderClient for ChatCompletionsProvider {
    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata {
            provider: Arc::from(self.config.provider.clone()),
            model: self.config.model.clone(),
        }
    }

    async fn stream_response(&self, request: ProviderRequest) -> Result<ProviderStream, String> {
        let response = self
            .client
            .post(format!(
                "{}/chat/completions",
                self.config.base_url.trim_end_matches('/')
            ))
            .bearer_auth(&self.config.api_key)
            .json(&chat_body(&self.config, request))
            .send()
            .await
            .map_err(|error| error.to_string())?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!(
                "{} chat completions request failed: {status} {body}",
                self.config.provider
            ));
        }
        Ok(Box::pin(parse_sse(response.bytes_stream())))
    }
}

fn chat_body(config: &ChatCompletionsProviderConfig, request: ProviderRequest) -> Value {
    let mut body = Map::from_iter([
        ("model".to_string(), json!(request.model)),
        ("messages".to_string(), messages(&request)),
        ("stream".to_string(), json!(true)),
    ]);

    if let Some(tools) = request.tools {
        body.insert("tools".to_string(), json!(map_tools(tools)));
    }
    if let Some(thinking) = config
        .thinking
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        body.insert("thinking".to_string(), json!({ "type": thinking }));
    }
    if let Some(reasoning_effort) = config
        .reasoning_effort
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        body.insert("reasoning_effort".to_string(), json!(reasoning_effort));
    }
    if let Some(max_tokens) = config.max_tokens {
        body.insert("max_tokens".to_string(), json!(max_tokens));
    }

    Value::Object(body)
}

fn messages(request: &ProviderRequest) -> Value {
    let mut messages = Vec::new();
    if let Some(instructions) = request
        .instructions
        .as_ref()
        .filter(|instructions| !instructions.trim().is_empty())
    {
        messages.push(json!({
            "role": "system",
            "content": instructions,
        }));
    }
    for message in &request.input {
        match message {
            ProviderMessage::Text { role, content } => messages.push(json!({
                "role": role,
                "content": content,
            })),
            ProviderMessage::ToolCalls { calls } => messages.push(json!({
                "role": "assistant",
                "content": Value::Null,
                "tool_calls": calls
                    .iter()
                    .map(|call| {
                        json!({
                            "id": call.call_id,
                            "type": "function",
                            "function": {
                                "name": call.name,
                                "arguments": call.arguments_raw,
                            },
                        })
                    })
                    .collect::<Vec<_>>(),
            })),
            ProviderMessage::ToolResult { call_id, content } => messages.push(json!({
                "role": "tool",
                "tool_call_id": call_id,
                "content": content,
            })),
        }
    }
    if let Some(outputs) = &request.function_call_outputs {
        messages.extend(tool_messages(outputs));
    }
    json!(messages)
}

fn tool_messages(outputs: &[FunctionCallOutput]) -> Vec<Value> {
    let mut messages = Vec::new();
    for group in output_groups(outputs) {
        let mut assistant = Map::from_iter([
            ("role".to_string(), json!("assistant")),
            (
                "content".to_string(),
                group
                    .assistant_content
                    .filter(|content| !content.is_empty())
                    .map(Value::String)
                    .unwrap_or(Value::Null),
            ),
            (
                "tool_calls".to_string(),
                json!(
                    group
                        .outputs
                        .iter()
                        .map(|output| {
                            json!({
                                "id": output.call_id,
                                "type": "function",
                                "function": {
                                    "name": output.call.name,
                                    "arguments": output.call.arguments_raw,
                                },
                            })
                        })
                        .collect::<Vec<_>>()
                ),
            ),
        ]);
        if let Some(reasoning_content) = group
            .reasoning_content
            .filter(|content| !content.is_empty())
        {
            assistant.insert(
                "reasoning_content".to_string(),
                Value::String(reasoning_content),
            );
        }
        messages.push(Value::Object(assistant));
        messages.extend(group.outputs.iter().map(|output| {
            json!({
                "role": "tool",
                "tool_call_id": output.call_id,
                "content": output.output,
            })
        }));
    }
    messages
}

struct OutputGroup<'a> {
    outputs: Vec<&'a FunctionCallOutput>,
    assistant_content: Option<String>,
    reasoning_content: Option<String>,
}

fn output_groups(outputs: &[FunctionCallOutput]) -> Vec<OutputGroup<'_>> {
    let mut groups: Vec<OutputGroup<'_>> = Vec::new();
    for output in outputs {
        let response_id = output.call.response_id.as_str();
        let current_response_id = groups
            .last()
            .and_then(|group| group.outputs.first())
            .map(|first| first.call.response_id.as_str());
        if current_response_id != Some(response_id) {
            groups.push(OutputGroup {
                outputs: Vec::new(),
                assistant_content: output.assistant_content.clone(),
                reasoning_content: output.reasoning_content.clone(),
            });
        }
        let group = groups.last_mut().expect("output group");
        if group.assistant_content.is_none() {
            group.assistant_content = output.assistant_content.clone();
        }
        if group.reasoning_content.is_none() {
            group.reasoning_content = output.reasoning_content.clone();
        }
        group.outputs.push(output);
    }
    groups
}

fn map_tools(tools: Vec<ProviderTool>) -> Vec<Value> {
    tools
        .into_iter()
        .map(|tool| match tool {
            ProviderTool::Function(tool) => json!({
                "type": "function",
                "function": {
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.parameters,
                },
            }),
        })
        .collect()
}

fn parse_sse(
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
            while let Some(index) = buffer.find('\n') {
                let line = buffer[..index].trim_end_matches('\r').to_string();
                buffer = buffer[index + 1..].to_string();
                if let Some(payload) = line.strip_prefix("data: ") {
                    if payload == "[DONE]" {
                        continue;
                    }
                    let events = parse_event(payload, &mut response_id, &mut accumulator)?;
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
