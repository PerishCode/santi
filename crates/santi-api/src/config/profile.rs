use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderConfig {
    OpenAiResponses(OpenAiResponsesConfig),
    ChatCompletions(ChatCompletionsConfig),
}

impl ProviderConfig {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::OpenAiResponses(_) => "openai_responses",
            Self::ChatCompletions(_) => "chat_completions",
        }
    }

    pub fn model(&self) -> &str {
        match self {
            Self::OpenAiResponses(config) => &config.model,
            Self::ChatCompletions(config) => &config.model,
        }
    }

    pub fn bytes(&self) -> usize {
        match self {
            Self::OpenAiResponses(config) => config.bytes,
            Self::ChatCompletions(config) => config.bytes,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenAiResponsesConfig {
    pub api_key: String,
    pub model: String,
    pub base_url: String,
    pub reasoning_effort: Option<String>,
    pub summary: Option<String>,
    pub max_output_tokens: Option<u32>,
    pub bytes: usize,
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
    pub bytes: usize,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Profile {
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
        summary: Option<String>,
        #[serde(default)]
        max_output_tokens: Option<u32>,
        #[serde(default)]
        bytes: Option<usize>,
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
        #[serde(default)]
        bytes: Option<usize>,
    },
}

pub fn resolve_provider_config(
    provider: &str,
    profile: &Profile,
) -> Result<ProviderConfig, String> {
    match profile {
        Profile::OpenaiResponses { .. } => resolve_openai(provider, profile),
        Profile::ChatCompletions { .. } => resolve_chat_completions(provider, profile),
    }
}

pub(super) fn resolve_openai(provider: &str, profile: &Profile) -> Result<ProviderConfig, String> {
    let Profile::OpenaiResponses {
        api_key,
        model,
        base_url,
        reasoning_effort,
        summary,
        max_output_tokens,
        bytes,
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
        summary: optional_profile_string(summary, provider, "reasoning_summary")?,
        max_output_tokens: *max_output_tokens,
        bytes: required_positive_usize(*bytes, provider, "bytes")?,
    }))
}

pub(super) fn resolve_chat_completions(
    provider: &str,
    profile: &Profile,
) -> Result<ProviderConfig, String> {
    let Profile::ChatCompletions {
        api_key,
        model,
        base_url,
        thinking,
        reasoning_effort,
        max_tokens,
        bytes,
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
        bytes: required_positive_usize(*bytes, provider, "bytes")?,
    }))
}

pub(super) fn required_profile_string(
    value: &Option<String>,
    provider: &str,
    field: &str,
) -> Result<String, String> {
    optional_profile_string(value, provider, field)?
        .ok_or_else(|| format!("provider {provider} field {field} is required"))
}

pub(super) fn required_positive_usize(
    value: Option<usize>,
    provider: &str,
    field: &str,
) -> Result<usize, String> {
    let value = value.ok_or_else(|| format!("provider {provider} field {field} is required"))?;
    if value == 0 {
        return Err(format!(
            "provider {provider} field {field} must be greater than zero"
        ));
    }
    Ok(value)
}

pub(super) fn optional_profile_string(
    value: &Option<String>,
    provider: &str,
    field: &str,
) -> Result<Option<String>, String> {
    resolve_value(value, provider, field)
}

pub(super) fn resolve_value(
    value: &Option<String>,
    provider: &str,
    field: &str,
) -> Result<Option<String>, String> {
    let Some(raw) = value.as_deref().and_then(trim_string) else {
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

pub fn optional_env(name: &str) -> Option<String> {
    env::var(name).ok().and_then(|value| trim_string(&value))
}

pub fn santi_home() -> PathBuf {
    if let Some(home) = optional_env("SANTI_HOME") {
        return expand_home(&home);
    }
    let base = env::var("HOME")
        .or_else(|_| env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(base).join(".santi")
}

#[derive(Debug, Clone)]
pub struct RuntimePaths {
    pub database: PathBuf,
    pub runtime: PathBuf,
    pub execution: PathBuf,
}

pub(super) fn expand_home(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/")
        && let Ok(home) = env::var("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(path)
}

pub(super) fn trim_string(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}
