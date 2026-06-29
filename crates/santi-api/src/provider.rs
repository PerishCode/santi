use std::sync::Arc;

use santi_provider::{
    ChatCompletionsProvider, ChatCompletionsProviderConfig, OpenAIProvider, OpenAIProviderConfig,
    ProviderClient,
};

use crate::config::{ChatCompletionsConfig, OpenAiResponsesConfig, ProviderConfig};

pub fn from_config(config: ProviderConfig) -> Arc<dyn ProviderClient> {
    match config {
        ProviderConfig::OpenAiResponses(config) => openai_provider(config),
        ProviderConfig::ChatCompletions(config) => chat_completions_provider(config),
    }
}

fn openai_provider(config: OpenAiResponsesConfig) -> Arc<dyn ProviderClient> {
    Arc::new(OpenAIProvider::new(OpenAIProviderConfig {
        api_key: config.api_key,
        model: config.model,
        base_url: config.base_url,
        reasoning_effort: config.reasoning_effort,
        reasoning_summary: config.reasoning_summary,
        max_output_tokens: config.max_output_tokens,
    }))
}

fn chat_completions_provider(config: ChatCompletionsConfig) -> Arc<dyn ProviderClient> {
    Arc::new(ChatCompletionsProvider::new(
        ChatCompletionsProviderConfig {
            provider: config.provider,
            api_key: config.api_key,
            model: config.model,
            base_url: config.base_url,
            thinking: config.thinking,
            reasoning_effort: config.reasoning_effort,
            max_tokens: config.max_tokens,
        },
    ))
}
