use santi_provider::{Item, Request, Tool};
use serde_json::{Value, json};

use crate::budget;

const ESTIMATOR: &str = "provider_json_bytes_v1";

pub(crate) fn estimate_provider_request(request: &Request) -> budget::Estimate {
    estimate_provider_parts(
        &request.input,
        request.instructions.as_deref(),
        request.tools.as_deref(),
    )
}

pub(crate) fn estimate_provider_parts(
    input: &[Item],
    instructions: Option<&str>,
    tools: Option<&[Tool]>,
) -> budget::Estimate {
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
    budget::Estimate {
        estimator: ESTIMATOR.to_string(),
        items: input.len() as i64,
        input: held as i64,
        instructions: told as i64,
        tools: armed as i64,
        total: (held + told + armed) as i64,
    }
}

pub(crate) fn inbound_provider_item(
    kind: &crate::message::Kind,
    content: &crate::message::Content,
) -> Option<Item> {
    let text = content.rendered();
    if text.trim().is_empty() {
        return None;
    }
    let role = match kind {
        crate::message::Kind::Text => "user",
        crate::message::Kind::SantiSystem => "system",
    };
    Some(Item::Message {
        role: role.to_string(),
        content: text,
    })
}

fn provider_item_value(item: &Item) -> Value {
    match item {
        Item::Message { role, content } => json!({
            "type": "message",
            "role": role,
            "content": content,
        }),
        Item::Reasoning { id, content } => json!({
            "type": "reasoning",
            "id": id,
            "content": content,
        }),
        Item::Call {
            call,
            name,
            raw,
            item,
            mark,
        } => json!({
            "type": "function_call",
            "call": call,
            "name": name,
            "raw": raw,
            "item": item,
            "mark": mark,
        }),
        Item::Output { call, output } => json!({
            "type": "function_call_output",
            "call": call,
            "output": output,
        }),
    }
}

fn json_len(value: &Value) -> usize {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .unwrap_or(0)
}
