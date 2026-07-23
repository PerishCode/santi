use async_trait::async_trait;
use futures_core::Stream;
use reqwest::Client;
use serde::Deserialize;
use std::sync::Arc;

use crate::{ProviderClient, ProviderMetadata, ProviderRequest, ProviderStream};

#[derive(Debug, Clone)]
pub struct OpenAIProviderConfig {
    pub api_key: String,
    pub model: String,
    pub base_url: String,
    pub reasoning_effort: Option<String>,
    pub reasoning_summary: Option<String>,
    pub max_output_tokens: Option<u32>,
    pub bytes: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct OpenAIProvider {
    config: OpenAIProviderConfig,
    client: Client,
}

impl OpenAIProvider {
    pub fn new(config: OpenAIProviderConfig) -> Self {
        Self {
            config,
            client: Client::new(),
        }
    }
}

#[async_trait]
impl ProviderClient for OpenAIProvider {
    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata {
            provider: Arc::from("openai"),
            model: self.config.model.clone(),
            context_budget: self.config.bytes.map(|bytes| crate::ProviderContextBudget {
                bytes,
                source: "provider_config".to_string(),
            }),
        }
    }

    async fn stream_response(&self, request: ProviderRequest) -> Result<ProviderStream, String> {
        let response = self
            .client
            .post(format!(
                "{}/responses",
                self.config.base_url.trim_end_matches('/')
            ))
            .bearer_auth(&self.config.api_key)
            .json(&response_body(&self.config, request))
            .send()
            .await
            .map_err(|error| error.to_string())?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("openai responses request failed: {status} {body}"));
        }
        Ok(Box::pin(parse_sse(response.bytes_stream())))
    }
}

mod body;
mod stream;
use body::*;
use stream::*;
