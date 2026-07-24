use std::sync::Arc;

use santi_provider::{Provider, chat::completions, openai};

use crate::config::{ChatCompletions, OpenAiResponses, Resolved};

pub fn build(config: Resolved) -> Arc<dyn Provider> {
    match config {
        Resolved::OpenAiResponses(config) => responses(config),
        Resolved::ChatCompletions(config) => completions(config),
    }
}

fn responses(config: OpenAiResponses) -> Arc<dyn Provider> {
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

fn completions(config: ChatCompletions) -> Arc<dyn Provider> {
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
