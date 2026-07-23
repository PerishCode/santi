pub mod auth;
pub mod cli;
pub mod client;
pub mod config;
mod text;
pub mod watch;

use anyhow::Result;
use clap::Parser;

use auth::{Credentials, resolve_edge_bearer};
use cli::{Cli, ClientDefaults, Command, InboxCommand};
use client::run_client;
pub use text::source::read_inbox_seed_text;

pub async fn run() -> Result<()> {
    dotenvy::dotenv().ok();
    let cli = Cli::parse();
    match cli.command {
        Command::Service { args } => run_service(args).await,
        Command::Doctor { storage_only } => run_doctor(storage_only),
        Command::Inbox(inbox) => run_inbox(inbox, cli.strand),
        Command::Upgrade {
            deb,
            previous_deb,
            run,
            finalize,
        } => run_upgrade(deb, previous_deb, run, finalize),
        other => {
            let defaults = ClientDefaults {
                strand: cli.strand,
                soul: cli.soul,
            };
            let bearer = resolve_edge_bearer(Credentials {
                endpoint: cli.auth_token_url.as_deref(),
                identity: cli.auth_client_id.as_deref(),
                username: cli.auth_username.as_deref(),
                password: cli.auth_password.as_deref(),
                key: cli.api_key.as_deref(),
            })
            .await?;
            run_client(&cli.base_url, bearer.as_deref(), &defaults, other).await
        }
    }
}

fn run_doctor(storage_only: bool) -> Result<()> {
    config::boot(None, Default::default()).map_err(|error| anyhow::anyhow!(error))?;
    let report = if storage_only {
        santi_api::runtime::held().paths.doctor()
    } else {
        santi_api::ops::doctor()
    }
    .map_err(|error| anyhow::anyhow!(error))?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    if !report.ok {
        anyhow::bail!("doctor: unhealthy (see report above)");
    }
    Ok(())
}

fn run_inbox(command: InboxCommand, default_strand: Option<String>) -> Result<()> {
    match command {
        InboxCommand::Seed { text, file, stdin } => {
            let strand = default_strand
                .map(|id| id.trim().to_string())
                .filter(|id| !id.is_empty())
                .ok_or_else(|| anyhow::anyhow!("no strand id: set --strand / SANTI_STRAND_ID"))?;
            let text = read_inbox_seed_text(text, file, stdin)?;
            config::boot(None, Default::default()).map_err(|error| anyhow::anyhow!(error))?;
            let report = santi_api::ops::inbox_seed(&strand, &text)
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

fn run_upgrade(
    deb: Option<String>,
    previous_deb: Option<String>,
    run: bool,
    finalize: bool,
) -> Result<()> {
    config::boot(None, Default::default()).map_err(|error| anyhow::anyhow!(error))?;
    if finalize {
        let request = serde_json::from_reader(std::io::stdin().lock())?;
        let report = santi_api::upgrade::finalize(request).map_err(|error| {
            eprintln!(
                "{}",
                serde_json::to_string(&error).unwrap_or_else(|_| error.to_string())
            );
            anyhow::anyhow!(error)
        })?;
        println!("{}", serde_json::to_string(&report)?);
        Ok(())
    } else if run {
        let report = santi_api::upgrade::run(deb, previous_deb).map_err(|error| {
            eprintln!(
                "{}",
                serde_json::to_string(&error).unwrap_or_else(|_| error.to_string())
            );
            anyhow::anyhow!(error)
        })?;
        let terminal_error = report.errors.first().cloned();
        println!("{}", serde_json::to_string_pretty(&report)?);
        match terminal_error {
            Some(error) => Err(anyhow::anyhow!(error)),
            None => Ok(()),
        }
    } else {
        let deb = deb.ok_or_else(|| {
            anyhow::anyhow!("usage: santi upgrade <deb> [--previous-deb <current.deb>] [--run]")
        })?;
        let started =
            santi_api::upgrade::launch(&deb, previous_deb.as_deref()).map_err(|error| {
                eprintln!(
                    "{}",
                    serde_json::to_string(&error).unwrap_or_else(|_| error.to_string())
                );
                anyhow::anyhow!(error)
            })?;
        println!("{}", serde_json::to_string_pretty(&started)?);
        Ok(())
    }
}

async fn run_service(args: Vec<String>) -> Result<()> {
    let argv = std::iter::once("santi".to_string()).chain(args);
    let cli = config::ServiceCli::try_parse_from(argv)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    match cli.command.unwrap_or(config::ServiceCommand::Serve) {
        config::ServiceCommand::Serve => {
            config::boot(cli.config.as_deref(), cli.over.partial())
                .map_err(|error| anyhow::anyhow!(error))?;
            santi_api::serve()
                .await
                .map_err(|error| anyhow::anyhow!(error))
        }
        config::ServiceCommand::ExportOpenApi => {
            let document =
                santi_api::export_openapi_json().map_err(|error| anyhow::anyhow!(error))?;
            println!("{document}");
            Ok(())
        }
    }
}
