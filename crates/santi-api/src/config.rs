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
        let provider = trim_optional_string(&self.cli.provider)
            .or_else(|| trim_optional_string(&config.provider))
            .or_else(|| optional_env("SANTI_PROVIDER"))
            .unwrap_or_else(|| "openai".to_string());
        let profile = config
            .providers
            .get(&provider)
            .ok_or_else(|| format!("provider {provider} is not defined in {config_path}"))?;
        resolve_provider_config(&provider, profile)
    }

    fn config_path(&self) -> String {
        trim_optional_string(&self.cli.config)
            .or_else(|| optional_env("SANTI_CONFIG"))
            .unwrap_or_else(|| santi_home().join(APP_CONFIG_PATH).display().to_string())
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderConfig {
    OpenAiResponses(OpenAiResponsesConfig),
    ChatCompletions(ChatCompletionsConfig),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenAiResponsesConfig {
    pub api_key: String,
    pub model: String,
    pub base_url: String,
    pub reasoning_effort: Option<String>,
    pub reasoning_summary: Option<String>,
    pub max_output_tokens: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatCompletionsConfig {
    pub provider: String,
    pub api_key: String,
    pub model: String,
    pub base_url: String,
    pub thinking: Option<String>,
    pub reasoning_effort: Option<String>,
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct AppConfigFile {
    #[serde(default)]
    provider: Option<String>,
    providers: BTreeMap<String, RawProviderProfile>,
}

impl AppConfigFile {
    fn read(path: &str) -> Result<Self, String> {
        let content = fs::read_to_string(path)
            .map_err(|error| format!("failed to read app config {path}: {error}"))?;
        toml::from_str(&content)
            .map_err(|error| format!("failed to parse app config {path}: {error}"))
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum RawProviderProfile {
    OpenaiResponses {
        #[serde(default)]
        api_key: Option<String>,
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        base_url: Option<String>,
        #[serde(default)]
        reasoning_effort: Option<String>,
        #[serde(default)]
        reasoning_summary: Option<String>,
        #[serde(default)]
        max_output_tokens: Option<u32>,
    },
    ChatCompletions {
        #[serde(default)]
        api_key: Option<String>,
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        base_url: Option<String>,
        #[serde(default)]
        thinking: Option<String>,
        #[serde(default)]
        reasoning_effort: Option<String>,
        #[serde(default)]
        max_tokens: Option<u32>,
    },
}

fn resolve_provider_config(
    provider: &str,
    profile: &RawProviderProfile,
) -> Result<ProviderConfig, String> {
    match profile {
        RawProviderProfile::OpenaiResponses { .. } => resolve_openai(provider, profile),
        RawProviderProfile::ChatCompletions { .. } => resolve_chat_completions(provider, profile),
    }
}

fn resolve_openai(provider: &str, profile: &RawProviderProfile) -> Result<ProviderConfig, String> {
    let RawProviderProfile::OpenaiResponses {
        api_key,
        model,
        base_url,
        reasoning_effort,
        reasoning_summary,
        max_output_tokens,
    } = profile
    else {
        unreachable!("openai profile")
    };
    Ok(ProviderConfig::OpenAiResponses(OpenAiResponsesConfig {
        api_key: required_profile_string(api_key, provider, "api_key")?,
        model: required_profile_string(model, provider, "model")?,
        base_url: optional_profile_string(base_url, provider, "base_url")?
            .unwrap_or_else(|| "https://api.openai.com/v1".to_string()),
        reasoning_effort: optional_profile_string(reasoning_effort, provider, "reasoning_effort")?,
        reasoning_summary: optional_profile_string(
            reasoning_summary,
            provider,
            "reasoning_summary",
        )?,
        max_output_tokens: *max_output_tokens,
    }))
}

fn resolve_chat_completions(
    provider: &str,
    profile: &RawProviderProfile,
) -> Result<ProviderConfig, String> {
    let RawProviderProfile::ChatCompletions {
        api_key,
        model,
        base_url,
        thinking,
        reasoning_effort,
        max_tokens,
    } = profile
    else {
        unreachable!("chat completions profile")
    };
    Ok(ProviderConfig::ChatCompletions(ChatCompletionsConfig {
        provider: provider.to_string(),
        api_key: required_profile_string(api_key, provider, "api_key")?,
        model: required_profile_string(model, provider, "model")?,
        base_url: required_profile_string(base_url, provider, "base_url")?,
        thinking: optional_profile_string(thinking, provider, "thinking")?,
        reasoning_effort: optional_profile_string(reasoning_effort, provider, "reasoning_effort")?,
        max_tokens: *max_tokens,
    }))
}

fn required_profile_string(
    value: &Option<String>,
    provider: &str,
    field: &str,
) -> Result<String, String> {
    optional_profile_string(value, provider, field)?
        .ok_or_else(|| format!("provider {provider} field {field} is required"))
}

fn optional_profile_string(
    value: &Option<String>,
    provider: &str,
    field: &str,
) -> Result<Option<String>, String> {
    resolve_value(value, provider, field)
}

/// Resolve a config value that may be an `env://VAR` reference. A plain value is
/// used literally; `env://VAR` reads VAR from the environment. This is the same
/// `scheme://locator` vocabulary as the `session://` / `soul://` workspace URIs —
/// one indirection convention, so a toml can carry secrets as `env://` references
/// while the real values live only in the process environment. Fail-closed: an
/// `env://` reference to an unset/empty variable is an error, never silently empty.
fn resolve_value(
    value: &Option<String>,
    provider: &str,
    field: &str,
) -> Result<Option<String>, String> {
    let Some(raw) = trim_optional_string(value) else {
        return Ok(None);
    };
    let Some(var) = raw.strip_prefix("env://") else {
        return Ok(Some(raw));
    };
    let var = var.trim();
    if var.is_empty() {
        return Err(format!(
            "provider {provider} field {field}: env:// reference is missing a variable name"
        ));
    }
    optional_env(var).map(Some).ok_or_else(|| {
        format!("provider {provider} field {field} references env://{var}, which is unset or empty")
    })
}

fn optional_env(name: &str) -> Option<String> {
    env::var(name).ok().and_then(|value| trim_string(&value))
}

/// The santi home directory: `SANTI_HOME` if set, else `~/.santi`. It anchors
/// the default config path and runtime/execution/db locations, so santi runs
/// with zero explicit configuration. Explicit flags/env always override.
pub(crate) fn santi_home() -> PathBuf {
    if let Some(home) = optional_env("SANTI_HOME") {
        return expand_home(&home);
    }
    let base = env::var("HOME")
        .or_else(|_| env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(base).join(".santi")
}

fn expand_home(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/")
        && let Ok(home) = env::var("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(path)
}

fn trim_optional_string(value: &Option<String>) -> Option<String> {
    value.as_ref().and_then(|value| trim_string(value))
}

fn trim_string(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}
