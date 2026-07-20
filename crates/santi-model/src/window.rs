use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::Timestamp;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WindowSendRequest {
    pub content: String,
    pub client_message_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WindowSendAccepted {
    pub status: String,
    pub message_id: String,
    pub client_message_id: String,
    pub cursor: Option<i64>,
    pub received_at: Timestamp,
    pub receipt_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WindowTranscript {
    pub participant: String,
    pub entries: Vec<WindowTranscriptEntry>,
    pub next_since: i64,
    pub has_more: bool,
    pub empty: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WindowTranscriptEntry {
    pub message_id: String,
    pub seq: i64,
    pub author: WindowAuthor,
    pub text: String,
    pub at: Timestamp,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WindowAuthor {
    Human,
    Assistant,
}
