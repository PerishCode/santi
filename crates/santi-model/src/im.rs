use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::{IngestReceipt, Timestamp};

pub const IM_LABEL_PREFIX: &str = "im:";

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ImSendRequest {
    pub soul_id: String,
    pub participant_id: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ImSendResponse {
    pub participant_id: String,
    pub receipt: IngestReceipt,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ImParticipant {
    pub id: String,
    pub kind: String,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImDeliveryMode {
    Explicit,
    Automatic,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ImInboxEntry {
    pub seq: i64,
    pub id: String,
    pub participant_id: String,
    pub from_ref: Option<String>,
    pub turn_id: Option<String>,
    pub message_id: Option<String>,
    pub delivery_mode: Option<ImDeliveryMode>,
    pub content: String,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ImDelivery {
    pub seq: i64,
    pub id: String,
    pub participant_id: String,
    pub strand_id: String,
    pub turn_id: String,
    pub message_id: Option<String>,
    pub delivery_mode: ImDeliveryMode,
    pub created_at: Timestamp,
}
