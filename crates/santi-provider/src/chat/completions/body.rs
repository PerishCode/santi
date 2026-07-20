use serde_json::{Map, Value, json};

use crate::{ProviderItem, ProviderRequest, ProviderTool};

use super::*;

pub(super) fn chat_body(config: &ChatCompletionsProviderConfig, request: ProviderRequest) -> Value {
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
    for item in &request.input {
        if let Some(message) = message(item) {
            messages.push(message);
        }
    }
    json!(messages)
}

fn message(item: &ProviderItem) -> Option<Value> {
    match item {
        ProviderItem::Message { role, content } => Some(text_message(role, content)),
        ProviderItem::Reasoning { .. } => None,
        ProviderItem::FunctionCall {
            call_id,
            name,
            arguments_raw,
            ..
        } => Some(call_message(call_id, name, arguments_raw)),
        ProviderItem::FunctionCallOutput { call_id, output } => {
            Some(output_message(call_id, output))
        }
    }
}

fn text_message(role: &str, content: &str) -> Value {
    json!({ "role": role, "content": content })
}

fn call_message(call: &str, name: &str, arguments: &str) -> Value {
    json!({
        "role": "assistant",
        "content": Value::Null,
        "tool_calls": [{
            "id": call,
            "type": "function",
            "function": {
                "name": name,
                "arguments": arguments,
            },
        }],
    })
}

fn output_message(call: &str, output: &str) -> Value {
    json!({ "role": "tool", "tool_call_id": call, "content": output })
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
