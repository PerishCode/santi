//! Library implementation for the `santi` runtime and HTTP client binary.

pub mod auth;
pub mod cli;
pub mod client;
pub mod text_source;
pub mod watch;

use anyhow::Result;
use clap::Parser;

use auth::resolve_edge_bearer;
use cli::{Cli, ClientDefaults, Command, ImCommand, InboxCommand};
use client::run_client;
use text_source::{read_im_reply_text, read_inbox_seed_text};

pub async fn run() -> Result<()> {
    dotenvy::dotenv_override().ok();
    let cli = Cli::parse();
    match cli.command {
        Command::Service { args } => run_service(args).await,
        Command::Doctor => run_doctor(),
        Command::Inbox(inbox) => run_inbox(inbox, cli.strand),
        Command::Upgrade { deb, run } => run_upgrade(deb, run),
        // The soul's IM reply is an offline store write (like `inbox seed`) — no
        // HTTP, so a mid-turn reply never re-enters the turn-holding server.
        Command::Im(ImCommand::Reply { text, file, stdin }) => {
            let text = read_im_reply_text(text, file, stdin)?;
            run_im_reply(text, cli.strand)
        }
        other => {
            let defaults = ClientDefaults {
                strand: cli.strand,
                soul: cli.soul,
            };
            let bearer = resolve_edge_bearer(
                cli.auth_token_url.as_deref(),
                cli.auth_client_id.as_deref(),
                cli.auth_username.as_deref(),
                cli.auth_password.as_deref(),
                cli.api_key.as_deref(),
            )
            .await?;
            run_client(&cli.base_url, bearer.as_deref(), &defaults, other).await
        }
    }
}

/// Offline pre-check (local ops, no HTTP). Prints the report as JSON to stdout
/// and exits non-zero when unhealthy, so a caller (the upgrade flow) can gate.
fn run_doctor() -> Result<()> {
    let report = santi_api::ops::doctor().map_err(|error| anyhow::anyhow!(error))?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    if !report.ok {
        anyhow::bail!("doctor: unhealthy (see report above)");
    }
    Ok(())
}

/// Offline inbox producer (local ops, no HTTP). Resolves the strand from
/// --strand/SANTI_STRAND_ID and seeds a durable record; exits non-zero if the
/// inbox gate rejects it, so the upgrade flow notices a badly-behind strand.
fn run_inbox(command: InboxCommand, default_strand: Option<String>) -> Result<()> {
    match command {
        InboxCommand::Seed { text, file, stdin } => {
            let strand_id = default_strand
                .map(|id| id.trim().to_string())
                .filter(|id| !id.is_empty())
                .ok_or_else(|| anyhow::anyhow!("no strand id: set --strand / SANTI_STRAND_ID"))?;
            let text = read_inbox_seed_text(text, file, stdin)?;
            let report = santi_api::ops::inbox_seed(&strand_id, &text)
                .map_err(|error| anyhow::anyhow!(error))?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            if !report.accepted {
                anyhow::bail!(
                    "inbox seed rejected: {}",
                    report
                        .error
                        .map(|error| error.to_string())
                        .unwrap_or_default()
                );
            }
            Ok(())
        }
    }
}

/// The soul's IM reply egress (local ops, no HTTP). Resolves the current IM
/// conversation from --strand/SANTI_STRAND_ID (ambient in the soul's shell) and
/// delivers the reply into that conversation's participant inbox — a direct store
/// write, so a mid-turn reply never re-enters the turn-holding server.
fn run_im_reply(text: String, default_strand: Option<String>) -> Result<()> {
    let strand_id = default_strand
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!("no strand id: set --strand / SANTI_STRAND_ID (the IM conversation)")
        })?;
    let report =
        santi_api::ops::im_reply(&strand_id, &text).map_err(|error| anyhow::anyhow!(error))?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

/// Self-upgrade (local ops, no HTTP). `--run` executes the orchestration (what
/// the oneshot unit calls); otherwise it launches that unit detached and returns
/// the fast signal (监听 / 最长超时 Xmin / 日志位置).
fn run_upgrade(deb: Option<String>, run: bool) -> Result<()> {
    if run {
        let report = santi_api::upgrade::run(deb).map_err(|error| anyhow::anyhow!(error))?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        Ok(())
    } else {
        let deb = deb.ok_or_else(|| anyhow::anyhow!("usage: santi upgrade <deb> [--run]"))?;
        let started = santi_api::upgrade::launch(&deb).map_err(|error| anyhow::anyhow!(error))?;
        println!("{}", serde_json::to_string_pretty(&started)?);
        Ok(())
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
