use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    OpenAiResponses(OpenAiResponses),
    ChatCompletions(ChatCompletions),
}

impl Resolved {
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
pub struct OpenAiResponses {
    pub api_key: String,
    pub model: String,
    pub base_url: String,
    pub reasoning_effort: Option<String>,
    pub summary: Option<String>,
    pub max_output_tokens: Option<u32>,
    pub bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatCompletions {
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
    #[serde(rename = "openai_responses")]
    OpenAiResponses {
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

impl Profile {
    pub fn resolve(&self, provider: &str) -> Result<Resolved, String> {
        match self {
            Profile::OpenAiResponses {
                api_key,
                model,
                base_url,
                reasoning_effort,
                summary,
                max_output_tokens,
                bytes,
            } => Ok(Resolved::OpenAiResponses(OpenAiResponses {
                api_key: required(api_key, provider, "api_key")?,
                model: required(model, provider, "model")?,
                base_url: optional(base_url, provider, "base_url")?
                    .unwrap_or_else(|| "https://api.openai.com/v1".to_string()),
                reasoning_effort: optional(reasoning_effort, provider, "reasoning_effort")?,
                summary: optional(summary, provider, "reasoning_summary")?,
                max_output_tokens: *max_output_tokens,
                bytes: positive(*bytes, provider, "bytes")?,
            })),
            Profile::ChatCompletions {
                api_key,
                model,
                base_url,
                thinking,
                reasoning_effort,
                max_tokens,
                bytes,
            } => Ok(Resolved::ChatCompletions(ChatCompletions {
                provider: provider.to_string(),
                api_key: required(api_key, provider, "api_key")?,
                model: required(model, provider, "model")?,
                base_url: required(base_url, provider, "base_url")?,
                thinking: optional(thinking, provider, "thinking")?,
                reasoning_effort: optional(reasoning_effort, provider, "reasoning_effort")?,
                max_tokens: *max_tokens,
                bytes: positive(*bytes, provider, "bytes")?,
            })),
        }
    }
}

fn required(value: &Option<String>, provider: &str, field: &str) -> Result<String, String> {
    optional(value, provider, field)?
        .ok_or_else(|| format!("provider {provider} field {field} is required"))
}

fn positive(value: Option<usize>, provider: &str, field: &str) -> Result<usize, String> {
    let value = value.ok_or_else(|| format!("provider {provider} field {field} is required"))?;
    if value == 0 {
        return Err(format!(
            "provider {provider} field {field} must be greater than zero"
        ));
    }
    Ok(value)
}

fn optional(value: &Option<String>, provider: &str, field: &str) -> Result<Option<String>, String> {
    let Some(raw) = value.as_deref().and_then(trimmed) else {
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
    env(var).map(Some).ok_or_else(|| {
        format!("provider {provider} field {field} references env://{var}, which is unset or empty")
    })
}

pub fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().and_then(|value| trimmed(&value))
}

pub fn home() -> PathBuf {
    if let Some(home) = env("SANTI_HOME") {
        return expanded(&home);
    }
    let base = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(base).join(".santi")
}

#[derive(Debug, Clone)]
pub struct Layout {
    pub database: PathBuf,
    pub runtime: PathBuf,
    pub execution: PathBuf,
}

fn expanded(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/")
        && let Ok(home) = std::env::var("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(path)
}

fn trimmed(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}
