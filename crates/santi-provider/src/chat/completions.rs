use async_trait::async_trait;
use futures_core::Stream;
use reqwest::Client;
use std::sync::Arc;

use crate::{ProviderClient, ProviderMetadata, ProviderRequest, ProviderStream};

#[derive(Debug, Clone)]
pub struct ChatCompletionsProviderConfig {
    pub provider: String,
    pub api_key: String,
    pub model: String,
    pub base_url: String,
    pub thinking: Option<String>,
    pub reasoning_effort: Option<String>,
    pub max_tokens: Option<u32>,
    pub input_budget_bytes: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct ChatCompletionsProvider {
    config: ChatCompletionsProviderConfig,
    client: Client,
}

impl ChatCompletionsProvider {
    pub fn new(config: ChatCompletionsProviderConfig) -> Self {
        Self {
            config,
            client: Client::new(),
        }
    }
}

#[async_trait]
impl ProviderClient for ChatCompletionsProvider {
    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata {
            provider: Arc::from(self.config.provider.clone()),
            model: self.config.model.clone(),
            context_budget: self.config.input_budget_bytes.map(|input_budget_bytes| {
                crate::ProviderContextBudget {
                    input_budget_bytes,
                    source: "provider_config".to_string(),
                }
            }),
        }
    }

    async fn stream_response(&self, request: ProviderRequest) -> Result<ProviderStream, String> {
        let response = self
            .client
            .post(format!(
                "{}/chat/completions",
                self.config.base_url.trim_end_matches('/')
            ))
            .bearer_auth(&self.config.api_key)
            .json(&chat_body(&self.config, request))
            .send()
            .await
            .map_err(|error| error.to_string())?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!(
                "{} chat completions request failed: {status} {body}",
                self.config.provider
            ));
        }
        Ok(Box::pin(parse_sse(response.bytes_stream())))
    }
}

mod body;
mod stream;
use body::*;
use stream::*;
