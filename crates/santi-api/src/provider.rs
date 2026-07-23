use std::sync::Arc;

use santi_provider::{Provider, chat::completions, openai};

use crate::config::{ChatCompletionsConfig, OpenAiResponsesConfig, ProviderConfig};

pub fn from_config(config: ProviderConfig) -> Arc<dyn Provider> {
    match config {
        ProviderConfig::OpenAiResponses(config) => openai_provider(config),
        ProviderConfig::ChatCompletions(config) => chat_completions_provider(config),
    }
}

fn openai_provider(config: OpenAiResponsesConfig) -> Arc<dyn Provider> {
    Arc::new(openai::OpenAI::new(openai::Config {
        key: config.api_key,
        model: config.model,
        url: config.base_url,
        effort: config.reasoning_effort,
        summary: config.summary,
        ceiling: config.max_output_tokens,
        bytes: Some(config.bytes),
    }))
}

fn chat_completions_provider(config: ChatCompletionsConfig) -> Arc<dyn Provider> {
    Arc::new(completions::Chat::new(completions::Config {
        provider: config.provider,
        key: config.api_key,
        model: config.model,
        url: config.base_url,
        thinking: config.thinking,
        effort: config.reasoning_effort,
        ceiling: config.max_tokens,
        bytes: Some(config.bytes),
    }))
}
