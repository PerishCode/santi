use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ingest::Request)]
pub struct Request {
    pub soul: String,
    pub label: String,
    pub text: String,
    pub request: String,
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ingest::Receipt)]
pub struct Receipt {
    pub strand: String,
    pub inbox: String,
    pub warning: Option<Box<santi_error::Fault>>,
}

#[derive(Debug, Clone)]
pub enum Outcome {
    Accepted { receipt: Receipt },
    Rejected { error: Box<santi_error::Fault> },
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ingest::Source)]
pub struct Source {
    pub kind: String,
    pub source: Option<String>,
    pub metadata: Option<Value>,
}

impl Source {
    pub fn new(kind: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            source: None,
            metadata: None,
        }
    }

    pub fn with_ref(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    pub fn with_metadata(mut self, metadata: Value) -> Self {
        self.metadata = Some(metadata);
        self
    }
}
