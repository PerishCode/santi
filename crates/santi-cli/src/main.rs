//! `santi-cli` is a thin HTTP wrapper around `santi-api`.
//!
//! It deliberately exposes only the raw HTTP surface: every command maps to a
//! single `santi-api` endpoint, the CLI never links `santi-core`, and the only
//! way it touches the runtime is over HTTP. Responses are printed verbatim as
//! JSON so the CLI stays a transport, not a second source of truth.

use std::io::Write as _;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use futures_util::StreamExt;

const DEFAULT_BASE_URL: &str = "http://127.0.0.1:43307";

#[derive(Parser)]
#[command(name = "santi-cli", about = "HTTP wrapper around santi-api")]
struct Cli {
    /// Base URL of a running santi-api. Falls back to SANTI_API_URL, then the
    /// local default.
    #[arg(long, global = true, env = "SANTI_API_URL", default_value = DEFAULT_BASE_URL)]
    base_url: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// GET /api/v1/health
    Health,
    /// Session resources under /api/v1/sessions
    #[command(subcommand)]
    Session(SessionCommand),
}

#[derive(Subcommand)]
enum SessionCommand {
    /// POST /api/v1/sessions
    Create,
    /// GET /api/v1/sessions
    List,
    /// GET /api/v1/sessions/{id}
    Get { id: String },
    /// GET /api/v1/sessions/{id}/messages
    Messages { id: String },
    /// GET /api/v1/sessions/{id}/runtime
    Runtime { id: String },
    /// POST /api/v1/sessions/{id}/send
    Send { id: String, text: String },
    /// GET /api/v1/sessions/{id}/events (server-sent events, follows the stream)
    Events { id: String },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let client = reqwest::Client::new();
    let base = cli.base_url.trim_end_matches('/').to_string();

    match cli.command {
        Command::Health => get(&client, &format!("{base}/api/v1/health")).await,
        Command::Session(SessionCommand::Create) => {
            post(&client, &format!("{base}/api/v1/sessions"), None).await
        }
        Command::Session(SessionCommand::List) => {
            get(&client, &format!("{base}/api/v1/sessions")).await
        }
        Command::Session(SessionCommand::Get { id }) => {
            get(&client, &format!("{base}/api/v1/sessions/{id}")).await
        }
        Command::Session(SessionCommand::Messages { id }) => {
            get(&client, &format!("{base}/api/v1/sessions/{id}/messages")).await
        }
        Command::Session(SessionCommand::Runtime { id }) => {
            get(&client, &format!("{base}/api/v1/sessions/{id}/runtime")).await
        }
        Command::Session(SessionCommand::Send { id, text }) => {
            let body = serde_json::json!({
                "content": [{ "type": "text", "text": text }]
            });
            post(
                &client,
                &format!("{base}/api/v1/sessions/{id}/send"),
                Some(body),
            )
            .await
        }
        Command::Session(SessionCommand::Events { id }) => {
            follow(&client, &format!("{base}/api/v1/sessions/{id}/events")).await
        }
    }
}

async fn get(client: &reqwest::Client, url: &str) -> Result<()> {
    let response = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("GET {url}"))?;
    print_json(response).await
}

async fn post(client: &reqwest::Client, url: &str, body: Option<serde_json::Value>) -> Result<()> {
    let mut request = client.post(url);
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

/// Stream a server-sent-event endpoint, writing raw bytes through as they
/// arrive. The CLI does not parse or model the events; it is a pipe.
async fn follow(client: &reqwest::Client, url: &str) -> Result<()> {
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
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("read event stream")?;
        stdout.write_all(&chunk).context("write event chunk")?;
        stdout.flush().ok();
    }
    Ok(())
}
