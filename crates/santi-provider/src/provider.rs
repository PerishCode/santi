use async_trait::async_trait;
use futures_core::Stream;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{pin::Pin, sync::Arc};

/// A single typed item in the provider's input timeline — the convergent
/// "content block / item" shape (OpenAI Responses items ≈ Anthropic content
/// blocks ≈ Gemini parts). The domain models this superset; chat_completions is
/// a downward compatibility adapter that flattens it into a messages array.
#[derive(Debug, Clone)]
pub enum ProviderItem {
    /// A plain role/content message (user, assistant text, or system).
    Message { role: String, content: String },
    /// An assistant reasoning span. Currently summary text only (the encrypted
    /// reasoning payload is deferred until a Responses reasoning model is live);
    /// chat_completions drops it (GLM has no hard requirement — see DC5).
    Reasoning { id: Option<String>, content: String },
    /// An assistant function call. `item` is the provider's raw item (replayed
    /// verbatim by the Responses adapter); chat_completions rebuilds tool_calls
    /// from name/arguments_raw.
    FunctionCall {
        call_id: String,
        name: String,
        arguments_raw: String,
        item: Option<Value>,
        item_id: Option<String>,
    },
    /// The result of a function call, replayed back to the model.
    FunctionCallOutput { call_id: String, output: String },
}

#[derive(Debug, Clone)]
pub struct ProviderRequest {
    pub model: String,
    pub instructions: Option<String>,
    pub input: Vec<ProviderItem>,
    pub tools: Option<Vec<ProviderTool>>,
    /// Soft transport cache for the Responses adapter (server continuation).
    /// Dormant for now: always `None`, the adapter full-replays from `input`
    /// (the DB timeline stays the single source of truth). See DC2.
    pub previous_response_id: Option<String>,
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
