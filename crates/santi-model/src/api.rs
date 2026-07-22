use santi_error::SantiError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use super::*;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DownstreamCredential {
    pub id: String,
    pub label_prefix: String,
    pub credential_env: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateDownstreamRequest {
    pub id: String,
    pub label_prefix: String,
    pub credential_env: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct IngestRequest {
    pub soul_id: String,
    pub label: String,
    pub text: String,
    #[serde(default)]
    pub source_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TurnEvent {
    pub id: String,
    pub strand_id: String,
    pub turn_id: String,
    pub external_label: String,
    pub final_text: String,
    pub completed_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TurnEventPage {
    pub cursor: i64,
    pub event: TurnEvent,
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
    pub soul_id: String,
    pub strand_strategy: String,
    pub secret_env: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateWebhookRequest {
    pub name: String,
    pub adaptor: String,
    pub soul_id: String,
    #[serde(default)]
    pub strand_strategy: Option<String>,
    pub secret_env: String,
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
        .content_text()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SendStrandAcceptedResponse {
    pub strand: Strand,
    pub receipt: IngestReceipt,
    pub turn: Option<Turn>,
    pub user_message: Option<StrandMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct IngestReceipt {
    pub strand_id: String,
    pub inbox_id: String,
    pub warning: Option<Box<SantiError>>,
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
    pub turn_id: Option<String>,
    pub incident_id: Option<String>,
    pub reconstructed_from: Option<String>,
    pub occurred_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ReceiptStatus {
    pub inbox_id: String,
    pub strand_id: String,
    pub state: ReceiptState,
    pub accepted_at: Timestamp,
    pub updated_at: Timestamp,
    pub transitions: Vec<ReceiptTransition>,
    pub effects: Vec<StrandEffect>,
    pub im_deliveries: Vec<ImDelivery>,
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
    pub strand_id: String,
    pub state: DriveStrandState,
    pub turn: Option<Turn>,
}

#[derive(Debug, Clone)]
pub enum StrandSelector {
    ById(String),
    ByLabel { soul_id: String, label: String },
}

#[derive(Debug, Clone)]
pub enum IngestOutcome {
    Accepted { receipt: IngestReceipt },
    Rejected { error: Box<SantiError> },
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct InboxSource {
    pub source_type: String,
    pub source_ref: Option<String>,
    pub metadata: Option<Value>,
}

impl InboxSource {
    pub fn new(source_type: impl Into<String>) -> Self {
        Self {
            source_type: source_type.into(),
            source_ref: None,
            metadata: None,
        }
    }

    pub fn with_ref(mut self, source_ref: impl Into<String>) -> Self {
        self.source_ref = Some(source_ref.into());
        self
    }

    pub fn with_metadata(mut self, metadata: Value) -> Self {
        self.metadata = Some(metadata);
        self
    }
}
