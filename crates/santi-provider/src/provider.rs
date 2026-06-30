use async_trait::async_trait;
use futures_core::Stream;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{pin::Pin, sync::Arc};

#[derive(Debug, Clone)]
pub enum ProviderMessage {
    /// A plain role/content message (user, assistant text, or system).
    Text { role: String, content: String },
    /// A historical assistant turn that requested one or more tool calls.
    ToolCalls { calls: Vec<ProviderHistoricalCall> },
    /// A historical tool result, replayed back to the model.
    ToolResult { call_id: String, content: String },
}

#[derive(Debug, Clone)]
pub struct ProviderHistoricalCall {
    pub call_id: String,
    pub name: String,
    pub arguments_raw: String,
}

#[derive(Debug, Clone)]
pub struct ProviderRequest {
    pub model: String,
    pub instructions: Option<String>,
    pub input: Vec<ProviderMessage>,
    pub tools: Option<Vec<ProviderTool>>,
    pub previous_response_id: Option<String>,
    pub function_call_outputs: Option<Vec<FunctionCallOutput>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderTool {
    Function(ProviderFunctionTool),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderFunctionTool {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone)]
pub struct FunctionCallOutput {
    pub call: ProviderFunctionCall,
    pub call_id: String,
    pub output: String,
    pub assistant_content: Option<String>,
    pub reasoning_content: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderFunctionCall {
    pub response_id: String,
    pub item_id: Option<String>,
    pub item: Value,
    pub call_id: String,
    pub name: String,
    pub arguments_raw: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderStreamTrace {
    Chunk {
        bytes: usize,
    },
    RawEvent {
        raw_type: String,
        mapped_events: Vec<String>,
    },
}

#[derive(Debug, Clone)]
pub struct ProviderMetadata {
    pub provider: Arc<str>,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProviderEvent {
    StreamTrace(ProviderStreamTrace),
    ResponseStarted {
        provider_response_id: Option<String>,
    },
    ResponseInProgress {
        provider_response_id: Option<String>,
    },
    ReasoningSummaryDelta(String),
    ReasoningSummaryDone(String),
    TextDelta(String),
    FunctionCallRequested(ProviderFunctionCall),
    Completed {
        provider_response_id: Option<String>,
    },
    Failed(String),
}

pub type ProviderStream =
    Pin<Box<dyn Stream<Item = Result<ProviderEvent, String>> + Send + 'static>>;

#[async_trait]
pub trait ProviderClient: Send + Sync {
    fn metadata(&self) -> ProviderMetadata;

    async fn stream_response(&self, request: ProviderRequest) -> Result<ProviderStream, String>;
}
