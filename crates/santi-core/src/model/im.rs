use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::{IngestReceipt, Timestamp};

/// The external-label prefix that marks a strand as an IM conversation. The IM
/// layer builds `im:<participant_id>` labels; reply routing strips this back to
/// the participant.
pub const IM_LABEL_PREFIX: &str = "im:";

/// IM inbound: a participant sends content to a soul. The sender's address is
/// IM envelope only, carried by the `im:<participant_id>` conversation label.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ImSendRequest {
    pub soul_id: String,
    pub participant_id: String,
    pub content: String,
}

/// Durable enqueue confirmation for an IM send. The soul may still be mid-turn.
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

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImDeliveryMode {
    /// The soul ran the offline early-reply command during its turn.
    Explicit,
    /// Turn completion delivered the final assistant message transactionally.
    Automatic,
}

/// One delivered message in a participant's passive inbox.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ImInboxEntry {
    pub seq: i64,
    pub id: String,
    pub participant_id: String,
    pub from_ref: Option<String>,
    /// Absent for legacy or operator-authored entries outside a provider turn.
    pub turn_id: Option<String>,
    /// Present when automatic delivery used a final assistant message.
    pub message_id: Option<String>,
    pub delivery_mode: Option<ImDeliveryMode>,
    pub content: String,
    pub created_at: Timestamp,
}

/// Content-free delivery evidence projected onto an accepted inbox receipt.
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
