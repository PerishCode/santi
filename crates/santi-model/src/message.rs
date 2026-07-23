use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use super::Timestamp;

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

    pub fn rendered(&self) -> String {
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
    pub role: ActorType,
    pub actor: String,
    pub kind: MessageKind,
    pub content: MessageContent,
    pub state: MessageState,
    pub version: i64,
    pub deleted: Option<Timestamp>,
    pub created: Timestamp,
    pub updated: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StrandMessageRef {
    pub strand: String,
    pub message: String,
    pub seq: i64,
    pub created: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StrandMessage {
    pub relation: StrandMessageRef,
    pub message: Message,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MessageEvent {
    pub id: String,
    pub message: String,
    pub action: String,
    pub role: ActorType,
    pub actor: String,
    pub base_version: i64,
    pub payload: Value,
    pub created: Timestamp,
}
