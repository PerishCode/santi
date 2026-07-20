use serde_json::{Map, Value, json};

use crate::{ProviderItem, ProviderRequest, ProviderTool};

use super::*;

pub(super) fn response_body(config: &OpenAIProviderConfig, request: ProviderRequest) -> Value {
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
    let items = request
        .input
        .iter()
        .filter_map(response_item)
        .collect::<Vec<_>>();
    json!(items)
}

fn response_item(item: &ProviderItem) -> Option<Value> {
    match item {
        ProviderItem::Message { role, content } => Some(response_message(role, content)),
        ProviderItem::Reasoning { .. } => None,
        ProviderItem::FunctionCall {
            call_id,
            name,
            arguments_raw,
            item,
            ..
        } => Some(response_call(call_id, name, arguments_raw, item)),
        ProviderItem::FunctionCallOutput { call_id, output } => {
            Some(response_output(call_id, output))
        }
    }
}

fn response_message(role: &str, content: &str) -> Value {
    let content_type = if role == "assistant" {
        "output_text"
    } else {
        "input_text"
    };
    json!({
        "role": role,
        "content": [{ "type": content_type, "text": content }],
    })
}

fn response_call(call: &str, name: &str, arguments: &str, item: &Option<Value>) -> Value {
    validated_function_call_replay(item).unwrap_or_else(|| {
        eprintln!(
            "santi-provider: ignored invalid openai replay cache for call {call}; regenerated from canonical event"
        );
        json!({
            "type": "function_call",
            "call_id": call,
            "name": name,
            "arguments": arguments,
        })
    })
}

fn response_output(call: &str, output: &str) -> Value {
    json!({ "type": "function_call_output", "call_id": call, "output": output })
}

fn validated_function_call_replay(item: &Option<Value>) -> Option<Value> {
    let item = item.as_ref()?;
    if item.get("type").and_then(Value::as_str) != Some("function_call") {
        return None;
    }
    if let Some(id) = item.get("id").and_then(Value::as_str)
        && !id.starts_with("fc")
    {
        return None;
    }
    Some(item.clone())
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
