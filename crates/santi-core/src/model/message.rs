use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use super::Timestamp;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActorType {
    Account,
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
pub struct SessionMessageRef {
    pub session_id: String,
    pub message_id: String,
    pub session_seq: i64,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SessionMessage {
    pub relation: SessionMessageRef,
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
