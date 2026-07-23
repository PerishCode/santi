use async_trait::async_trait;
use futures_core::Stream;
use reqwest::Client;
use serde::Deserialize;
use std::sync::Arc;

use crate::{Metadata, Provider, Request, Streaming};

#[derive(Debug, Clone)]
pub struct Config {
    pub key: String,
    pub model: String,
    pub url: String,
    pub effort: Option<String>,
    pub summary: Option<String>,
    pub ceiling: Option<u32>,
    pub bytes: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct OpenAI {
    config: Config,
    client: Client,
}

impl OpenAI {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            client: Client::new(),
        }
    }
}

#[async_trait]
impl Provider for OpenAI {
    fn metadata(&self) -> Metadata {
        Metadata {
            provider: Arc::from("openai"),
            model: self.config.model.clone(),
            budget: self.config.bytes.map(|bytes| crate::Cap {
                bytes,
                source: "provider_config".to_string(),
            }),
        }
    }

    async fn stream(&self, request: Request) -> Result<Streaming, String> {
        let response = self
            .client
            .post(format!(
                "{}/responses",
                self.config.url.trim_end_matches('/')
            ))
            .bearer_auth(&self.config.key)
            .json(&body(&self.config, request))
            .send()
            .await
            .map_err(|error| error.to_string())?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("openai responses request failed: {status} {body}"));
        }
        Ok(Box::pin(frames(response.bytes_stream())))
    }
}

mod body;
mod stream;
use body::*;
use stream::*;
