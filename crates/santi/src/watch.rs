use std::collections::HashSet;
use std::io::Write;
use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::{Stream, StreamExt};

use crate::cli::WatchFormat;

const WATCH_IDLE_GRACE: Duration = Duration::from_millis(1500);

pub(crate) struct Watch<'a> {
    pub(crate) client: &'a reqwest::Client,
    pub(crate) base: &'a str,
    pub(crate) strand: &'a str,
    pub(crate) initial: Option<String>,
    pub(crate) format: WatchFormat,
}

pub(crate) async fn watch_until_idle(watch: Watch<'_>) -> Result<()> {
    let url = format!("{}/api/v1/strands/{}/events", watch.base, watch.strand);
    let response = watch
        .client
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
    if let Some(turn) = watch.initial {
        inflight.insert(turn);
        seeded = true;
    }
    let mut stdout = std::io::stdout();
    loop {
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
            break;
        };
        write_frame(&mut stdout, watch.format, &event, &data);
        track_frame(&event, &data, &mut inflight, &mut seeded);
    }
    Ok(())
}

fn write_frame(stdout: &mut impl Write, format: WatchFormat, event: &str, data: &str) {
    let line = match format {
        WatchFormat::Raw if event != "stream_open" => Some(data.to_string()),
        WatchFormat::Filtered => render_watch_event(event, data),
        _ => None,
    };
    if let Some(line) = line {
        writeln!(stdout, "{line}").ok();
        stdout.flush().ok();
    }
}

fn track_frame(event: &str, data: &str, inflight: &mut HashSet<String>, seeded: &mut bool) {
    match event {
        "turn_started" => {
            if let Some(id) = json_field(data, &["payload", "turn", "id"]) {
                inflight.insert(id);
                *seeded = true;
            }
        }
        "turn_completed" | "turn_failed" => {
            if let Some(id) = json_field(data, &["payload", "turn_id"]) {
                inflight.remove(&id);
            }
        }
        _ => {}
    }
}

mod render;
pub use render::*;

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

pub fn json_field(data: &str, path: &[&str]) -> Option<String> {
    let mut value = serde_json::from_str::<serde_json::Value>(data).ok()?;
    for key in path {
        value = value.get(key)?.clone();
    }
    value.as_str().map(str::to_string)
}
