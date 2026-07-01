use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use super::Timestamp;

/// No user/account actor: santi is individual-first, not multi-tenant. All
/// inbound (a CLI send, a webhook event) arrives as `System` — the sender's
/// identity is metainfo carried in the content, opaque to core, not a distinct
/// actor kind. `(actor, message_kind)` is the full marker at the provider
/// boundary (see `message_to_provider_item`): Soul→assistant, System+Text→user
/// (world-inbound), System+SantiSystem→system (runtime-meta, not user speech).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActorType {
    Soul,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MessageState {
    Pending,
    Fixed,
    Aborted,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    Text,
    SantiSystem,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MessageContent {
    pub parts: Vec<MessagePart>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessagePart {
    Text {
        text: String,
    },
    Image {
        mime_type: String,
        data_base64: String,
    },
}

impl MessageContent {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            parts: vec![MessagePart::Text { text: text.into() }],
        }
    }

    pub fn content_text(&self) -> String {
        self.parts
            .iter()
            .filter_map(|part| match part {
                MessagePart::Text { text } => Some(text.as_str()),
                MessagePart::Image { .. } => None,
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

/// Whether an appended message is a REQUEST addressed to the soul (a user send /
/// webhook event → wakes the soul / drives a turn) or a RECORD of something that
/// happened (the soul's own output, a failure notice, a future compact → does not
/// wake). The drive trigger keys off this; see PHASE-04 (M3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageIntake {
    Request,
    Record,
}

impl MessageIntake {
    pub fn is_request(self) -> bool {
        matches!(self, MessageIntake::Request)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Message {
    pub id: String,
    pub actor_type: ActorType,
    pub actor_id: String,
    pub message_kind: MessageKind,
    pub content: MessageContent,
    pub state: MessageState,
    pub version: i64,
    pub deleted_at: Option<Timestamp>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StrandMessageRef {
    pub strand_id: String,
    pub message_id: String,
    pub strand_seq: i64,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StrandMessage {
    pub relation: StrandMessageRef,
    pub message: Message,
    pub content_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MessageEvent {
    pub id: String,
    pub message_id: String,
    pub action: String,
    pub actor_type: ActorType,
    pub actor_id: String,
    pub base_version: i64,
    pub payload: Value,
    pub created_at: Timestamp,
}
