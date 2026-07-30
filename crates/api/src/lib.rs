pub mod cli;
pub mod config;
pub mod text;

use anyhow::Result;
use clap::Parser;

use cli::{Capability, Cli, Command, InboxCommand, Job};

pub async fn run() -> Result<()> {
    config::load();
    let Cli {
        config,
        strand,
        over,
        command,
    } = Cli::parse();
    match command.unwrap_or(Command::Serve) {
        Command::Serve => {
            config::boot(config.as_deref(), over.partial()).map_err(anyhow::Error::msg)?;
            santi_api::serve().await.map_err(anyhow::Error::msg)
        }
        Command::Export => {
            let document = santi_api::export_openapi_json().map_err(anyhow::Error::msg)?;
            println!("{document}");
            Ok(())
        }
        Command::Doctor { storage_only } => {
            config::boot(config.as_deref(), over.partial()).map_err(anyhow::Error::msg)?;
            let report = if storage_only {
                santi_api::runtime::held().paths.doctor().await
            } else {
                santi_api::ops::doctor().await
            }
            .map_err(anyhow::Error::msg)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            if !report.ok {
                anyhow::bail!("doctor: unhealthy (see report above)");
            }
            Ok(())
        }
        Command::Audit {
            turn,
            failed,
            limit,
            after,
        } => {
            config::boot(config.as_deref(), over.partial()).map_err(anyhow::Error::msg)?;
            let rows = santi_api::runtime::held()
                .paths
                .audit(santi_api::ops::Audit {
                    strand: strand.as_deref(),
                    turn: turn.as_deref(),
                    failed,
                    limit,
                    after: after.as_deref(),
                })
                .await
                .map_err(anyhow::Error::msg)?;
            println!("{}", serde_json::to_string(&rows)?);
            Ok(())
        }
        Command::Inbox(InboxCommand::Seed { text, file, stdin }) => {
            let strand = strand
                .map(|id| id.trim().to_string())
                .filter(|id| !id.is_empty())
                .ok_or_else(|| anyhow::anyhow!("no strand id: set --strand / SANTI_STRAND_ID"))?;
            let text = text::read(text, file, stdin)?;
            config::boot(config.as_deref(), over.partial()).map_err(anyhow::Error::msg)?;
            let report = santi_api::ops::inbox_seed(&strand, &text)
                .await
                .map_err(anyhow::Error::msg)?;
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
        Command::Capability(Capability::Public) => {
            config::boot(config.as_deref(), over.partial()).map_err(anyhow::Error::msg)?;
            let issuer = santi_api::runtime::held()
                .capability
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("runtime capability authority is not configured"))?;
            println!(
                "{}",
                serde_json::json!({
                    "key_id": issuer.id(),
                    "public_key": issuer.public(),
                })
            );
            Ok(())
        }
        Command::Job(Job::Run) => santi_api::jobs::run().map_err(anyhow::Error::msg),
        Command::Job(Job::Finalize) => santi_api::jobs::finalize().map_err(anyhow::Error::msg),
    }
}
