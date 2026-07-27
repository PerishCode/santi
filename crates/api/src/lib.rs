pub mod cli;
pub mod config;
pub mod text;

use anyhow::Result;
use clap::Parser;

use cli::{Cli, Command, InboxCommand};

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
        Command::Doctor => {
            config::boot(config.as_deref(), over.partial()).map_err(anyhow::Error::msg)?;
            let report = santi_api::ops::doctor().map_err(anyhow::Error::msg)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            if !report.ok {
                anyhow::bail!("doctor: unhealthy (see report above)");
            }
            Ok(())
        }
        Command::Inbox(InboxCommand::Seed { text, file, stdin }) => {
            let strand = strand
                .map(|id| id.trim().to_string())
                .filter(|id| !id.is_empty())
                .ok_or_else(|| anyhow::anyhow!("no strand id: set --strand / SANTI_STRAND_ID"))?;
            let text = text::read(text, file, stdin)?;
            config::boot(config.as_deref(), over.partial()).map_err(anyhow::Error::msg)?;
            let report = santi_api::ops::inbox_seed(&strand, &text).map_err(anyhow::Error::msg)?;
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
