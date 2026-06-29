//! `santi` is the single entry point for the runtime.
//!
//! It has two faces, split by subcommand:
//!
//! - `santi service ...` runs the runtime server in-process (delegated to the
//!   `santi-api` crate, which owns the HTTP boundary and links `santi-core`).
//! - every other command is a transport-only HTTP client: it reaches the
//!   runtime exclusively over HTTP against a running server, never calling
//!   `santi-core` in-process. HTTP stays the only way into the runtime.

use std::io::Write as _;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use futures_util::StreamExt;

const DEFAULT_BASE_URL: &str = "http://127.0.0.1:43307";

#[derive(Parser)]
#[command(
    name = "santi",
    version,
    about = "santi runtime: server (`service`) and HTTP client"
)]
struct Cli {
    /// Base URL of a running santi server. Falls back to SANTI_API_URL, then
    /// the local default. Only used by the HTTP client commands.
    #[arg(long, global = true, env = "SANTI_API_URL", default_value = DEFAULT_BASE_URL)]
    base_url: String,

    /// Bearer token sent on client requests when the server requires one.
    /// Falls back to SANTI_API_KEY. Only used by the HTTP client commands.
    #[arg(long, global = true, env = "SANTI_API_KEY")]
    api_key: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the runtime server in-process (`serve`, `export-openapi`).
    Service {
        /// Arguments forwarded to the server (e.g. `serve`, `export-openapi`,
        /// `--config`, `--provider`).
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
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
    dotenvy::dotenv_override().ok();
    let cli = Cli::parse();
    match cli.command {
        Command::Service { args } => run_service(args).await,
        other => run_client(&cli.base_url, cli.api_key.as_deref(), other).await,
    }
}

/// Run the runtime server in-process via `santi-api`.
async fn run_service(args: Vec<String>) -> Result<()> {
    let argv = std::iter::once("santi".to_string()).chain(args);
    let config = santi_api::config::ConfigService::from_args(argv)
        .map_err(|error| anyhow::anyhow!(error))?;
    match config.command() {
        santi_api::config::AppCommand::Serve => santi_api::serve(config)
            .await
            .map_err(|error| anyhow::anyhow!(error)),
        santi_api::config::AppCommand::ExportOpenApi => {
            let document =
                santi_api::export_openapi_json().map_err(|error| anyhow::anyhow!(error))?;
            println!("{document}");
            Ok(())
        }
    }
}

/// Transport-only HTTP client against a running server.
async fn run_client(base_url: &str, api_key: Option<&str>, command: Command) -> Result<()> {
    let client = build_client(api_key)?;
    let base = base_url.trim_end_matches('/').to_string();
    match command {
        Command::Service { .. } => unreachable!("service is handled before the client path"),
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

/// Build an HTTP client that attaches `Authorization: Bearer <key>` to every
/// request when an api key is configured.
fn build_client(api_key: Option<&str>) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder();
    if let Some(key) = api_key {
        let mut headers = reqwest::header::HeaderMap::new();
        let mut value = reqwest::header::HeaderValue::from_str(&format!("Bearer {key}"))
            .context("invalid api key")?;
        value.set_sensitive(true);
        headers.insert(reqwest::header::AUTHORIZATION, value);
        builder = builder.default_headers(headers);
    }
    builder.build().context("build http client")
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
/// arrive. The client does not parse or model the events; it is a pipe.
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
