use anyhow::{Context, Result};

use super::REQUEST_TIMEOUT;
use crate::cli::WatchFormat;
use crate::watch::{Watch, watch_until_idle};

pub struct Request<'a> {
    pub client: &'a reqwest::Client,
    pub base: &'a str,
    pub strand: &'a str,
    pub body: serde_json::Value,
    pub watch: bool,
    pub format: WatchFormat,
}

pub async fn send(request: Request<'_>) -> Result<()> {
    let url = format!("{}/api/v1/strands/{}/send", request.base, request.strand);
    let response = request
        .client
        .post(&url)
        .timeout(REQUEST_TIMEOUT)
        .json(&request.body)
        .send()
        .await
        .with_context(|| format!("POST {url}"))?;
    let status = response.status();
    let text = response.text().await.context("read response body")?;
    let accepted = serde_json::from_str::<serde_json::Value>(&text).ok();
    if !request.watch {
        print_response(accepted.as_ref(), &text)?;
    }
    if !status.is_success() {
        if request.watch {
            println!("{text}");
        }
        anyhow::bail!("request failed with status {status}");
    }
    if let Some(warning) = accepted_warning(accepted.as_ref()) {
        if request.watch {
            print_response(accepted.as_ref(), &text)?;
        }
        return Err(accepted_warning_error(warning));
    }
    if !request.watch {
        return Ok(());
    }
    let seed_turn = accepted
        .as_ref()
        .and_then(|value| value.get("turn"))
        .and_then(|turn| turn.get("id"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    watch_until_idle(Watch {
        client: request.client,
        base: request.base,
        strand: request.strand,
        initial: seed_turn,
        format: request.format,
    })
    .await
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
