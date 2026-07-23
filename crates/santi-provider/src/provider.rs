use async_trait::async_trait;
use futures_core::Stream;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{pin::Pin, sync::Arc};

#[derive(Debug, Clone)]
pub enum Item {
    Message {
        role: String,
        content: String,
    },
    Reasoning {
        id: Option<String>,
        content: String,
    },
    Call {
        call: String,
        name: String,
        raw: String,
        item: Option<Value>,
        mark: Option<String>,
    },
    Output {
        call: String,
        output: String,
    },
}

#[derive(Debug, Clone)]
pub struct Request {
    pub model: String,
    pub instructions: Option<String>,
    pub input: Vec<Item>,
    pub tools: Option<Vec<Tool>>,
    pub previous: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tool {
    Function(Function),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Function {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    pub response: String,
    pub mark: Option<String>,
    pub item: Value,
    pub call: String,
    pub name: String,
    pub raw: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trace {
    Chunk { bytes: usize },
    Raw { kind: String, mapped: Vec<String> },
}

#[derive(Debug, Clone)]
pub struct Metadata {
    pub provider: Arc<str>,
    pub model: String,
    pub budget: Option<Cap>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cap {
    pub bytes: usize,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Traced(Trace),
    Started { response: Option<String> },
    Working { response: Option<String> },
    Thinking(String),
    Thought(String),
    Text(String),
    Called(Call),
    Completed { response: Option<String> },
    Failed(String),
}

pub type Streaming = Pin<Box<dyn Stream<Item = Result<Event, String>> + Send + 'static>>;

#[async_trait]
pub trait Provider: Send + Sync {
    fn metadata(&self) -> Metadata;

    async fn stream(&self, request: Request) -> Result<Streaming, String>;
}
