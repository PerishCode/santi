use std::{collections::BTreeMap, env, fs, path::PathBuf};

use clap::{Parser, Subcommand};
use serde::Deserialize;

const APP_CONFIG_PATH: &str = "santi.toml";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AppCommand {
    #[default]
    Serve,
    ExportOpenApi,
}

#[derive(Debug, Clone)]
pub struct ConfigService {
    cli: Cli,
}

impl ConfigService {
    pub fn from_env_args() -> Result<Self, String> {
        Cli::try_parse()
            .map(|cli| Self { cli })
            .map_err(|error| error.to_string())
    }

    pub fn from_args(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        Cli::try_parse_from(args)
            .map(|cli| Self { cli })
            .map_err(|error| error.to_string())
    }

    pub fn command(&self) -> AppCommand {
        match self.cli.command {
            Some(CliCommand::Serve) | None => AppCommand::Serve,
            Some(CliCommand::ExportOpenApi) => AppCommand::ExportOpenApi,
        }
    }

    pub fn provider_config(&self) -> Result<ProviderConfig, String> {
        let config_path = self.config_path();
        let config = AppConfigFile::read(&config_path)?;
        let provider = self.selected_provider(&config);
        let profile = config
            .providers
            .get(&provider)
            .ok_or_else(|| format!("provider {provider} is not defined in {config_path}"))?;
        resolve_provider_config(&provider, profile)
    }

    pub fn provider_name(&self) -> Result<String, String> {
        let config = AppConfigFile::read(&self.config_path())?;
        Ok(self.selected_provider(&config))
    }

    pub fn listen(&self) -> Result<plumb::config::Listen, String> {
        let config = AppConfigFile::read(&self.config_path())?;
        Ok(config.listen)
    }

    fn config_path(&self) -> String {
        self.cli
            .config
            .as_deref()
            .and_then(trim_string)
            .or_else(|| optional_env("SANTI_CONFIG"))
            .unwrap_or_else(|| santi_home().join(APP_CONFIG_PATH).display().to_string())
    }

    fn selected_provider(&self, config: &AppConfigFile) -> String {
        self.cli
            .provider
            .as_deref()
            .and_then(trim_string)
            .or_else(|| config.provider.as_deref().and_then(trim_string))
            .or_else(|| optional_env("SANTI_PROVIDER"))
            .unwrap_or_else(|| "openai".to_string())
    }
}

#[derive(Debug, Clone, Parser)]
#[command(disable_help_subcommand = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<CliCommand>,
    #[arg(long, global = true)]
    config: Option<String>,
    #[arg(long, global = true)]
    provider: Option<String>,
}

#[derive(Debug, Clone, Copy, Subcommand)]
enum CliCommand {
    Serve,
    #[command(name = "export-openapi")]
    ExportOpenApi,
}

mod profile;
pub use profile::*;
