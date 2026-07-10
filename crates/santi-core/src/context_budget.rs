use santi_provider::{ProviderItem, ProviderRequest, ProviderTool};
use serde_json::{Value, json};

use crate::ContextEstimate;

const ESTIMATOR: &str = "provider_json_bytes_v1";

pub(crate) fn estimate_provider_request(request: &ProviderRequest) -> ContextEstimate {
    estimate_provider_parts(
        &request.input,
        request.instructions.as_deref(),
        request.tools.as_deref(),
    )
}

pub(crate) fn estimate_provider_parts(
    input: &[ProviderItem],
    instructions: Option<&str>,
    tools: Option<&[ProviderTool]>,
) -> ContextEstimate {
    let input_bytes = input
        .iter()
        .map(provider_item_value)
        .map(|value| json_len(&value))
        .sum::<usize>();
    let instructions_bytes = instructions.map_or(0, |text| text.len());
    let tools_bytes = tools
        .map(|tools| {
            serde_json::to_vec(tools)
                .map(|bytes| bytes.len())
                .unwrap_or(0)
        })
        .unwrap_or(0);
    ContextEstimate {
        estimator: ESTIMATOR.to_string(),
        input_items: input.len() as i64,
        input_bytes: input_bytes as i64,
        instructions_bytes: instructions_bytes as i64,
        tools_bytes: tools_bytes as i64,
        total_bytes: (input_bytes + instructions_bytes + tools_bytes) as i64,
    }
}

pub(crate) fn inbound_provider_item(
    message_kind: &crate::MessageKind,
    content: &crate::MessageContent,
) -> Option<ProviderItem> {
    let text = content.content_text();
    if text.trim().is_empty() {
        return None;
    }
    let role = match message_kind {
        crate::MessageKind::Text => "user",
        crate::MessageKind::SantiSystem => "system",
    };
    Some(ProviderItem::Message {
        role: role.to_string(),
        content: text,
    })
}

fn provider_item_value(item: &ProviderItem) -> Value {
    match item {
        ProviderItem::Message { role, content } => json!({
            "type": "message",
            "role": role,
            "content": content,
        }),
        ProviderItem::Reasoning { id, content } => json!({
            "type": "reasoning",
            "id": id,
            "content": content,
        }),
        ProviderItem::FunctionCall {
            call_id,
            name,
            arguments_raw,
            item,
            item_id,
        } => json!({
            "type": "function_call",
            "call_id": call_id,
            "name": name,
            "arguments_raw": arguments_raw,
            "item": item,
            "item_id": item_id,
        }),
        ProviderItem::FunctionCallOutput { call_id, output } => json!({
            "type": "function_call_output",
            "call_id": call_id,
            "output": output,
        }),
    }
}

fn json_len(value: &Value) -> usize {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .unwrap_or(0)
}
