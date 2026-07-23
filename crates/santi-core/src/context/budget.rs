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
    let held = input
        .iter()
        .map(provider_item_value)
        .map(|value| json_len(&value))
        .sum::<usize>();
    let told = instructions.map_or(0, |text| text.len());
    let armed = tools
        .map(|tools| {
            serde_json::to_vec(tools)
                .map(|bytes| bytes.len())
                .unwrap_or(0)
        })
        .unwrap_or(0);
    ContextEstimate {
        estimator: ESTIMATOR.to_string(),
        items: input.len() as i64,
        input: held as i64,
        instructions: told as i64,
        tools: armed as i64,
        total: (held + told + armed) as i64,
    }
}

pub(crate) fn inbound_provider_item(
    kind: &crate::MessageKind,
    content: &crate::MessageContent,
) -> Option<ProviderItem> {
    let text = content.rendered();
    if text.trim().is_empty() {
        return None;
    }
    let role = match kind {
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
            mark,
        } => json!({
            "type": "function_call",
            "call_id": call_id,
            "name": name,
            "arguments_raw": arguments_raw,
            "item": item,
            "mark": mark,
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
