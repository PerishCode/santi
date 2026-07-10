use anyhow::{Context, Result};

use super::REQUEST_TIMEOUT;
use crate::cli::WatchFormat;
use crate::watch::watch_until_idle;

/// POST a send, then optionally watch the stream until the strand is idle.
pub async fn send(
    client: &reqwest::Client,
    base: &str,
    strand_id: &str,
    body: serde_json::Value,
    watch: bool,
    watch_format: WatchFormat,
) -> Result<()> {
    let url = format!("{base}/api/v1/strands/{strand_id}/send");
    let response = client
        .post(&url)
        .timeout(REQUEST_TIMEOUT)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("POST {url}"))?;
    let status = response.status();
    let text = response.text().await.context("read response body")?;
    let accepted = serde_json::from_str::<serde_json::Value>(&text).ok();
    if !watch {
        print_response(accepted.as_ref(), &text)?;
    }
    if !status.is_success() {
        if watch {
            println!("{text}");
        }
        anyhow::bail!("request failed with status {status}");
    }
    if let Some(warning) = accepted_warning(accepted.as_ref()) {
        if watch {
            print_response(accepted.as_ref(), &text)?;
        }
        return Err(accepted_warning_error(warning));
    }
    if !watch {
        return Ok(());
    }
    let seed_turn = accepted
        .as_ref()
        .and_then(|value| value.get("turn"))
        .and_then(|turn| turn.get("id"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    watch_until_idle(client, base, strand_id, seed_turn, watch_format).await
}

pub(super) fn accepted_warning(value: Option<&serde_json::Value>) -> Option<&serde_json::Value> {
    value?
        .pointer("/receipt/warning")
        .filter(|warning| !warning.is_null())
}

pub(super) fn accepted_warning_error(warning: &serde_json::Value) -> anyhow::Error {
    if let Some(command) = warning
        .pointer("/context/recovery/command")
        .and_then(serde_json::Value::as_str)
    {
        anyhow::anyhow!("message was accepted but not driven; do not resend it; run `{command}`")
    } else {
        let code = warning
            .get("code")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown warning");
        anyhow::anyhow!(
            "message was accepted but not driven; do not resend it; inspect and resolve `{code}`"
        )
    }
}

fn print_response(accepted: Option<&serde_json::Value>, text: &str) -> Result<()> {
    match accepted {
        Some(value) => println!("{}", serde_json::to_string_pretty(value)?),
        None => println!("{text}"),
    }
    Ok(())
}
