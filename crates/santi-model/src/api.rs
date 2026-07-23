use santi_error::Fault;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use super::*;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DownstreamCredential {
    pub id: String,
    pub prefix: String,
    #[serde(skip)]
    #[schema(ignore)]
    pub digest: String,
    pub created: Timestamp,
    pub updated: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateDownstreamRequest {
    pub id: String,
    pub prefix: String,
    pub digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct IngestRequest {
    pub soul: String,
    pub label: String,
    pub text: String,
    pub request: String,
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TurnEvent {
    pub id: String,
    pub strand: String,
    pub turn: String,
    pub label: String,
    pub text: String,
    pub completed: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TurnEventBatch {
    pub cursor: i64,
    pub events: Vec<TurnEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateSoulRequest {
    #[serde(default)]
    pub memory: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WebhookSubscription {
    pub name: String,
    pub adaptor: String,
    pub soul: String,
    pub strategy: String,
    pub credential: String,
    pub created: Timestamp,
    pub updated: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateWebhookRequest {
    pub name: String,
    pub adaptor: String,
    pub soul: String,
    #[serde(default)]
    pub strategy: Option<String>,
    pub credential: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateStrandResponse {
    pub strand: Strand,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ForkStrandResponse {
    pub strand: Strand,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StrandDetail {
    pub strand: Strand,
    pub messages: Vec<StrandMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SendStrandRequest {
    pub content: Vec<MessagePart>,
}

impl SendStrandRequest {
    pub fn text(&self) -> String {
        MessageContent {
            parts: self.content.clone(),
        }
        .rendered()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SendStrandAcceptedResponse {
    pub strand: Strand,
    pub receipt: IngestReceipt,
    pub turn: Option<Turn>,
    pub message: Option<StrandMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct IngestReceipt {
    pub strand: String,
    pub inbox: String,
    pub warning: Option<Box<Fault>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptState {
    Accepted,
    MechanicallyRecovered,
    Driving,
    TurnFailed,
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ReceiptTransition {
    pub id: String,
    pub sequence: i64,
    pub state: ReceiptState,
    pub turn: Option<String>,
    pub incident: Option<String>,
    pub rebuilt: Option<String>,
    pub occurred: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ReceiptStatus {
    pub inbox: String,
    pub strand: String,
    pub state: ReceiptState,
    pub accepted: Timestamp,
    pub updated: Timestamp,
    pub transitions: Vec<ReceiptTransition>,
    pub effects: Vec<StrandEffect>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DriveStrandState {
    Started,
    Running,
    Idle,
    Paused,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DriveStrandResponse {
    pub strand: String,
    pub state: DriveStrandState,
    pub turn: Option<Turn>,
}

#[derive(Debug, Clone)]
pub enum StrandSelector {
    ById(String),
    ByLabel { soul: String, label: String },
}

#[derive(Debug, Clone)]
pub enum IngestOutcome {
    Accepted { receipt: IngestReceipt },
    Rejected { error: Box<Fault> },
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct InboxSource {
    pub kind: String,
    pub source: Option<String>,
    pub metadata: Option<Value>,
}

impl InboxSource {
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
