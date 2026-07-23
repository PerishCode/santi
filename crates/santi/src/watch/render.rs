use super::*;

pub fn render_watch_event(event: &str, data: &str) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(data).ok()?;
    match event {
        "stream_open" | "message_delta" | "thinking_updated" => None,
        "turn_started" => {
            let turn = value.get("payload")?.get("turn")?;
            let id = turn.get("id").and_then(serde_json::Value::as_str)?;
            let trigger = turn
                .get("trigger_type")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown");
            Some(format!("turn started {id} ({trigger})"))
        }
        "turn_activity" => {
            let activity = value.get("payload")?.get("activity")?;
            let id = activity
                .get("turn_id")
                .and_then(serde_json::Value::as_str)?;
            let state = activity
                .get("state")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown");
            Some(format!("turn {id}: {state}"))
        }
        "message_created" => {
            let message = value.get("payload")?.get("message")?;
            Some(format!(
                "message {} {}: {}",
                message_seq(message),
                message_actor_kind(message),
                snippet(
                    message
                        .get("content_text")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default(),
                    160,
                )
            ))
        }
        "message_completed" => {
            let payload = value.get("payload")?;
            let turn_id = payload
                .get("turn_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown");
            let text = payload
                .get("message")
                .and_then(|message| message.get("content_text"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            Some(format!(
                "assistant completed {turn_id}: {}",
                snippet(text, 500)
            ))
        }
        "tool_call_created" => {
            let call = value.get("payload")?.get("tool_call")?;
            let id = call
                .get("id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown");
            let name = call
                .get("tool_name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("tool");
            Some(format!("tool call {name} ({id})"))
        }
        "tool_result_created" => {
            let result = value.get("payload")?.get("tool_result")?;
            let call_id = result
                .get("tool_call_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown");
            let status = if result
                .get("error_text")
                .is_some_and(|error| !error.is_null())
            {
                "error"
            } else {
                "ok"
            };
            Some(format!("tool result {call_id}: {status}"))
        }
        "thinking_created" => thinking_line(&value, "thinking started"),
        "thinking_completed" => thinking_line(&value, "thinking completed"),
        "material_updated" => {
            let material = value.get("payload")?.get("material")?;
            let kind = material
                .get("kind")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("material");
            Some(format!("material updated {kind}"))
        }
        "turn_completed" => {
            json_field(data, &["payload", "turn_id"]).map(|id| format!("turn completed {id}"))
        }
        "turn_failed" => render_turn_failure(value.get("payload")?),
        "error_transition" => render_error_transition(value.get("payload")?),
        _ => Some(format!("{event}: {}", snippet(data, 240))),
    }
}

fn render_turn_failure(payload: &serde_json::Value) -> Option<String> {
    let id = payload
        .get("turn_id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    let error = payload.get("error")?;
    let code = error
        .get("code")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    let message = error
        .get("message")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown error");
    let incident = error
        .get("incident")
        .and_then(serde_json::Value::as_str)
        .map(|id| format!(" (incident {id})"))
        .unwrap_or_default();
    Some(format!(
        "turn failed {id}: {}",
        snippet(&format!("{code}: {message}{incident}"), 240)
    ))
}

fn render_error_transition(payload: &serde_json::Value) -> Option<String> {
    let transition = payload.get("transition")?;
    let kind = transition
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("changed");
    let incident = transition.get("held")?;
    let id = incident
        .get("id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    let code = incident
        .get("code")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    Some(format!("error {kind} {code} ({id})"))
}

fn thinking_line(value: &serde_json::Value, label: &str) -> Option<String> {
    let thinking = value.get("payload")?.get("thinking")?;
    let id = thinking
        .get("id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    let turn_id = thinking
        .get("turn_id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    Some(format!("{label} {id} ({turn_id})"))
}

fn message_seq(message: &serde_json::Value) -> String {
    message
        .get("relation")
        .and_then(|relation| relation.get("strand_seq"))
        .and_then(serde_json::Value::as_i64)
        .map(|seq| format!("#{seq}"))
        .unwrap_or_else(|| "#?".to_string())
}

fn message_actor_kind(message: &serde_json::Value) -> String {
    let inner = message.get("message").unwrap_or(message);
    let actor = inner
        .get("actor_type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    let kind = inner
        .get("message_kind")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("text");
    format!("{actor}/{kind}")
}
