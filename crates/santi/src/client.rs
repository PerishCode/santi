use std::io::Write as _;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use futures_util::StreamExt;

use crate::cli::{
    ClientDefaults, Command, CompactCommand, ImCommand, StrandCommand, WatchFormat, split_send_args,
};
use crate::text_source::read_summary_file;
use crate::watch::{next_sse_frame, render_watch_event, watch_until_idle};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Transport-only HTTP client against a running server.
pub(crate) async fn run_client(
    base_url: &str,
    bearer: Option<&str>,
    defaults: &ClientDefaults,
    command: Command,
) -> Result<()> {
    let client = build_client(bearer)?;
    let base = base_url.trim_end_matches('/').to_string();
    match command {
        Command::Service { .. } => unreachable!("service is handled before the client path"),
        Command::Doctor => unreachable!("doctor is handled before the client path"),
        Command::Inbox(_) => unreachable!("inbox is handled before the client path"),
        Command::Upgrade { .. } => unreachable!("upgrade is handled before the client path"),
        Command::Health => get(&client, &format!("{base}/api/v1/health")).await,
        Command::Strand(StrandCommand::Create) => {
            post(&client, &format!("{base}/api/v1/strands"), None).await
        }
        Command::Strand(StrandCommand::List) => {
            get(&client, &format!("{base}/api/v1/strands")).await
        }
        Command::Strand(StrandCommand::Get { id }) => {
            let id = defaults.resolve_strand(id)?;
            get(&client, &format!("{base}/api/v1/strands/{id}")).await
        }
        Command::Strand(StrandCommand::Messages { id }) => {
            let id = defaults.resolve_strand(id)?;
            get(&client, &format!("{base}/api/v1/strands/{id}/messages")).await
        }
        Command::Strand(StrandCommand::Runtime { id }) => {
            let id = defaults.resolve_strand(id)?;
            get(&client, &format!("{base}/api/v1/strands/{id}/runtime")).await
        }
        Command::Strand(StrandCommand::Budget { id }) => {
            let id = defaults.resolve_strand(id)?;
            get(&client, &format!("{base}/api/v1/strands/{id}/budget")).await
        }
        Command::Strand(StrandCommand::Rejections { id, limit }) => {
            let id = defaults.resolve_strand(id)?;
            get(
                &client,
                &format!("{base}/api/v1/strands/{id}/rejections?limit={limit}"),
            )
            .await
        }
        Command::Strand(StrandCommand::Fork { id }) => {
            let id = defaults.resolve_strand(id)?;
            post(&client, &format!("{base}/api/v1/strands/{id}/fork"), None).await
        }
        Command::Strand(StrandCommand::Send {
            args,
            watch,
            watch_format,
        }) => {
            let (id, text) = split_send_args(args, defaults)?;
            let mut content = serde_json::json!({
                "content": [{ "type": "text", "text": text }]
            });
            if let Some(soul) = defaults.soul() {
                content["soul_id"] = serde_json::Value::from(soul);
            }
            send(&client, &base, &id, content, watch, watch_format).await
        }
        Command::Strand(StrandCommand::Events { id, format }) => {
            let id = defaults.resolve_strand(id)?;
            follow(
                &client,
                &format!("{base}/api/v1/strands/{id}/events"),
                format,
            )
            .await
        }
        Command::Im(ImCommand::Send {
            text,
            participant,
            reply,
            reply_timeout,
        }) => {
            let soul = defaults
                .soul()
                .ok_or_else(|| anyhow::anyhow!("no target soul: set --soul / SANTI_SOUL_ID"))?;
            let body = serde_json::json!({
                "soul_id": soul,
                "participant_id": participant,
                "content": text,
            });
            im_send(&client, &base, body, &participant, reply, reply_timeout).await
        }
        Command::Im(ImCommand::Poll { participant, since }) => {
            get(
                &client,
                &format!("{base}/api/v1/im/inbox/{participant}?since={since}"),
            )
            .await
        }
        Command::Im(ImCommand::Reply { .. }) => {
            unreachable!("im reply is handled before the client path")
        }
        Command::Compact(CompactCommand::Exec {
            from,
            to,
            summary,
            summary_file,
        }) => {
            let strand = defaults.resolve_strand(None)?;
            let summary = match summary_file {
                Some(path) => read_summary_file(&path)?,
                None => summary.expect("clap requires summary or summary_file"),
            };
            let mut body = serde_json::json!({
                "from_message_id": from,
                "to_message_id": to,
                "summary": summary,
            });
            if let Some(soul) = defaults.soul() {
                body["soul_id"] = serde_json::Value::from(soul);
            }
            post(
                &client,
                &format!("{base}/api/v1/strands/{strand}/compact"),
                Some(body),
            )
            .await
        }
        Command::Compact(CompactCommand::Capsule {
            from,
            to,
            from_seq,
            to_seq,
            summary,
            summary_file,
            source,
            reason,
            risk,
            queryability,
            dry_run,
        }) => {
            let body = compact_capsule_body(
                from,
                to,
                from_seq,
                to_seq,
                summary,
                summary_file,
                source,
                reason,
                risk,
                queryability,
                dry_run,
                defaults.soul(),
            )?;
            let strand = defaults.resolve_strand(None)?;
            post(
                &client,
                &format!("{base}/api/v1/strands/{strand}/compact"),
                Some(body),
            )
            .await
        }
        Command::Compact(CompactCommand::Query {
            compact_id,
            keyword,
            page_index,
            page_size,
        }) => {
            let mut url = format!(
                "{base}/api/v1/compacts/{compact_id}?page_index={page_index}&page_size={page_size}"
            );
            if let Some(keyword) = keyword.filter(|k| !k.is_empty()) {
                url.push_str(&format!("&keyword={}", urlencoding_encode(&keyword)));
            }
            get(&client, &url).await
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn compact_capsule_body(
    from: Option<String>,
    to: Option<String>,
    from_seq: Option<i64>,
    to_seq: Option<i64>,
    summary: Option<String>,
    summary_file: Option<String>,
    source: String,
    reason: String,
    risk: String,
    queryability: String,
    dry_run: bool,
    soul: Option<&str>,
) -> Result<serde_json::Value> {
    let summary = match summary_file {
        Some(path) => read_summary_file(&path)?,
        None => summary.expect("clap requires summary or summary_file"),
    };
    let mut body = serde_json::json!({
        "from_message_id": from,
        "to_message_id": to,
        "from_seq": from_seq,
        "to_seq": to_seq,
        "summary": summary,
        "capsule": {
            "source": source,
            "reason": reason,
            "risk": risk,
            "queryability": queryability,
        },
        "dry_run": dry_run,
    });
    if let Some(soul) = soul {
        body["soul_id"] = serde_json::Value::from(soul);
    }
    Ok(body)
}

fn urlencoding_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// Build an HTTP client that attaches a configured bearer to every request.
fn build_client(bearer: Option<&str>) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder();
    if let Some(token) = bearer {
        let mut headers = reqwest::header::HeaderMap::new();
        let mut value = reqwest::header::HeaderValue::from_str(&format!("Bearer {token}"))
            .context("invalid bearer token")?;
        value.set_sensitive(true);
        headers.insert(reqwest::header::AUTHORIZATION, value);
        builder = builder.default_headers(headers);
    }
    builder.build().context("build http client")
}

async fn get(client: &reqwest::Client, url: &str) -> Result<()> {
    let response = client
        .get(url)
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .with_context(|| format!("GET {url}"))?;
    print_json(response).await
}

async fn post(client: &reqwest::Client, url: &str, body: Option<serde_json::Value>) -> Result<()> {
    let mut request = client.post(url).timeout(REQUEST_TIMEOUT);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request
        .send()
        .await
        .with_context(|| format!("POST {url}"))?;
    print_json(response).await
}

async fn print_json(response: reqwest::Response) -> Result<()> {
    let status = response.status();
    let text = response.text().await.context("read response body")?;
    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(value) => println!("{}", serde_json::to_string_pretty(&value)?),
        Err(_) => println!("{text}"),
    }
    if !status.is_success() {
        anyhow::bail!("request failed with status {status}");
    }
    Ok(())
}

/// Stream a server-sent-event endpoint. Raw mode writes bytes through as before;
/// filtered mode parses frames and prints human-readable milestone lines.
async fn follow(client: &reqwest::Client, url: &str, format: WatchFormat) -> Result<()> {
    let response = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("GET {url}"))?;
    let status = response.status();
    if !status.is_success() {
        anyhow::bail!("request failed with status {status}");
    }
    let mut stream = response.bytes_stream();
    let mut stdout = std::io::stdout();
    match format {
        WatchFormat::Raw => {
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.context("read event stream")?;
                stdout.write_all(&chunk).context("write event chunk")?;
                stdout.flush().ok();
            }
        }
        WatchFormat::Filtered => {
            let mut buffer = String::new();
            while let Some((event, data)) = next_sse_frame(&mut stream, &mut buffer).await? {
                if let Some(line) = render_watch_event(&event, &data) {
                    writeln!(stdout, "{line}").ok();
                    stdout.flush().ok();
                }
            }
        }
    }
    Ok(())
}

/// IM send: POST the message, then (with --reply) poll the participant's inbox
/// for the soul's reply until it arrives or the timeout elapses. Baselines the
/// inbox high-water BEFORE sending so only the NEW reply is shown; on silence it
/// returns (the reply may still arrive later — the caller can poll it).
async fn im_send(
    client: &reqwest::Client,
    base: &str,
    body: serde_json::Value,
    participant: &str,
    reply: bool,
    reply_timeout: u64,
) -> Result<()> {
    let baseline = if reply {
        im_inbox_high_water(client, base, participant).await?
    } else {
        0
    };
    let url = format!("{base}/api/v1/im/send");
    let response = client
        .post(&url)
        .timeout(REQUEST_TIMEOUT)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("POST {url}"))?;
    let status = response.status();
    let text = response.text().await.context("read response body")?;
    if !status.is_success() {
        println!("{text}");
        anyhow::bail!("im send failed with status {status}");
    }
    if !reply {
        match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(value) => println!("{}", serde_json::to_string_pretty(&value)?),
            Err(_) => println!("{text}"),
        }
        return Ok(());
    }
    // Poll for the reply. A real turn runs minutes; poll gently past the baseline.
    let inbox_url = format!("{base}/api/v1/im/inbox/{participant}?since={baseline}");
    let deadline = Instant::now() + Duration::from_secs(reply_timeout);
    loop {
        let entries: serde_json::Value = client
            .get(&inbox_url)
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await
            .with_context(|| format!("GET {inbox_url}"))?
            .json()
            .await
            .context("parse inbox")?;
        if entries
            .as_array()
            .is_some_and(|entries| !entries.is_empty())
        {
            println!("{}", serde_json::to_string_pretty(&entries)?);
            return Ok(());
        }
        if Instant::now() >= deadline {
            eprintln!(
                "(no reply within {reply_timeout}s — it may still arrive; poll: santi im poll --as {participant} --since {baseline})"
            );
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// The current max `seq` in a participant's inbox — the baseline a `--reply` send
/// polls past, so it shows only the new reply, not the conversation history.
async fn im_inbox_high_water(
    client: &reqwest::Client,
    base: &str,
    participant: &str,
) -> Result<i64> {
    let url = format!("{base}/api/v1/im/inbox/{participant}?since=0");
    let entries: serde_json::Value = client
        .get(&url)
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .with_context(|| format!("GET {url}"))?
        .json()
        .await
        .context("parse inbox")?;
    Ok(entries
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("seq").and_then(serde_json::Value::as_i64))
        .max()
        .unwrap_or(0))
}

/// POST a send, then optionally `--watch` the stream until the strand is
/// idle again. Without `--watch` this is the prior fire-and-return behavior.
pub async fn send(
    client: &reqwest::Client,
    base: &str,
    strand_id: &str,
    body: serde_json::Value,
    watch: bool,
    watch_format: WatchFormat,
) -> Result<()> {
    let url = format!("{base}/api/v1/strands/{strand_id}/send");
    // The send itself returns as soon as the message is enqueued (it does not
    // wait for the turn), so it is an immediate request and gets the timeout;
    // the optional `--watch` below follows the SSE stream, which does not.
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
        match &accepted {
            Some(value) => println!("{}", serde_json::to_string_pretty(value)?),
            None => println!("{text}"),
        }
    }
    if !status.is_success() {
        if watch {
            println!("{text}");
        }
        anyhow::bail!("request failed with status {status}");
    }
    if !watch {
        return Ok(());
    }
    // Seed the in-flight set with the turn this send landed on (a fresh turn, or
    // the running one it coalesced into), so a follow-on that handles our message
    // is still awaited even if its `turn_started` arrives after the seed's end.
    let seed_turn = accepted
        .as_ref()
        .and_then(|value| value.get("turn"))
        .and_then(|turn| turn.get("id"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    watch_until_idle(client, base, strand_id, seed_turn, watch_format).await
}
