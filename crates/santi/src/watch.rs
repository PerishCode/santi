use std::collections::HashSet;
use std::io::Write as _;
use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::{Stream, StreamExt};

use crate::cli::WatchFormat;

const WATCH_IDLE_GRACE: Duration = Duration::from_millis(1500);

/// Follow the strand's SSE stream, tracking which turns are in flight, and
/// return once none remain (the strand has caught up). Filtered mode prints
/// milestone lines; raw mode preserves the prior compact JSON-line stream.
pub(crate) async fn watch_until_idle(
    client: &reqwest::Client,
    base: &str,
    strand_id: &str,
    seed_turn: Option<String>,
    format: WatchFormat,
) -> Result<()> {
    let url = format!("{base}/api/v1/strands/{strand_id}/events");
    let response = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("GET {url}"))?;
    let status = response.status();
    if !status.is_success() {
        anyhow::bail!("request failed with status {status}");
    }
    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    let mut inflight: HashSet<String> = HashSet::new();
    let mut seeded = false;
    if let Some(turn) = seed_turn {
        inflight.insert(turn);
        seeded = true;
    }
    let mut stdout = std::io::stdout();
    loop {
        // Once nothing is in flight, allow only a short grace for a coalesced
        // follow-on turn before declaring idle. Before seeding (no known turn),
        // wait without a deadline so we don't exit before the turn appears.
        let frame = if seeded && inflight.is_empty() {
            match tokio::time::timeout(WATCH_IDLE_GRACE, next_sse_frame(&mut stream, &mut buffer))
                .await
            {
                Ok(frame) => frame?,
                Err(_) => break,
            }
        } else {
            next_sse_frame(&mut stream, &mut buffer).await?
        };
        let Some((event, data)) = frame else {
            break; // stream closed
        };
        match format {
            WatchFormat::Raw if event != "stream_open" => {
                writeln!(stdout, "{data}").ok();
                stdout.flush().ok();
            }
            WatchFormat::Filtered => {
                if let Some(line) = render_watch_event(&event, &data) {
                    writeln!(stdout, "{line}").ok();
                    stdout.flush().ok();
                }
            }
            _ => {}
        }
        match event.as_str() {
            "turn_started" => {
                if let Some(id) = json_field(&data, &["payload", "turn", "id"]) {
                    inflight.insert(id);
                    seeded = true;
                }
            }
            "turn_completed" | "turn_failed" => {
                if let Some(id) = json_field(&data, &["payload", "turn_id"]) {
                    inflight.remove(&id);
                }
            }
            _ => {}
        }
    }
    Ok(())
}

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
        "turn_failed" => {
            let id = json_field(data, &["payload", "turn_id"]).unwrap_or_else(|| "unknown".into());
            let error =
                json_field(data, &["payload", "error"]).unwrap_or_else(|| "unknown error".into());
            Some(format!("turn failed {id}: {}", snippet(&error, 240)))
        }
        _ => Some(format!("{event}: {}", snippet(data, 240))),
    }
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

pub fn snippet(text: &str, limit: usize) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out = String::new();
    for ch in normalized.chars().take(limit) {
        out.push(ch);
    }
    if normalized.chars().count() > limit {
        out.push('…');
    }
    out
}

/// Pull the next complete SSE frame, returning its `(event, data)` lines.
/// Comment-only frames (keep-alives) and frames without an event are skipped.
/// Returns `Ok(None)` when the stream ends.
pub async fn next_sse_frame<B: AsRef<[u8]>>(
    stream: &mut (impl Stream<Item = reqwest::Result<B>> + Unpin),
    buffer: &mut String,
) -> Result<Option<(String, String)>> {
    loop {
        while let Some(boundary) = buffer.find("\n\n") {
            let frame: String = buffer.drain(..boundary + 2).collect();
            if let Some(parsed) = parse_sse_frame(&frame) {
                return Ok(Some(parsed));
            }
        }
        match stream.next().await {
            Some(chunk) => {
                let chunk = chunk.context("read event stream")?;
                buffer.push_str(&String::from_utf8_lossy(chunk.as_ref()));
            }
            None => return Ok(None),
        }
    }
}

/// Parse one SSE frame into `(event, data)`. Returns None if it has no event
/// line (e.g. a `:` keep-alive comment).
pub fn parse_sse_frame(frame: &str) -> Option<(String, String)> {
    let mut event = None;
    let mut data = String::new();
    for line in frame.lines() {
        if let Some(rest) = line.strip_prefix("event:") {
            event = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
        }
    }
    event.map(|event| (event, data))
}

/// Read a nested string field from a compact JSON document by key path.
pub fn json_field(data: &str, path: &[&str]) -> Option<String> {
    let mut value = serde_json::from_str::<serde_json::Value>(data).ok()?;
    for key in path {
        value = value.get(key)?.clone();
    }
    value.as_str().map(str::to_string)
}
