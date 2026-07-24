use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use crate::Timestamp;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[schema(as = message::Role)]
pub enum Role {
    Soul,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[schema(as = message::State)]
pub enum State {
    Pending,
    Fixed,
    Aborted,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[schema(as = message::Kind)]
pub enum Kind {
    Text,
    SantiSystem,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = message::Content)]
pub struct Content {
    pub parts: Vec<Part>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
#[schema(as = message::Part)]
pub enum Part {
    Text {
        text: String,
    },
    Image {
        mime_type: String,
        data_base64: String,
    },
}

impl Content {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            parts: vec![Part::Text { text: text.into() }],
        }
    }

    pub fn rendered(&self) -> String {
        self.parts
            .iter()
            .filter_map(|part| match part {
                Part::Text { text } => Some(text.as_str()),
                Part::Image { .. } => None,
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intake {
    Request,
    Record,
}

impl Intake {
    pub fn is_request(self) -> bool {
        matches!(self, Intake::Request)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = message::Message)]
pub struct Message {
    pub id: String,
    pub role: Role,
    pub actor: String,
    pub kind: Kind,
    pub content: Content,
    pub state: State,
    pub version: i64,
    pub deleted: Option<Timestamp>,
    pub created: Timestamp,
    pub updated: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = message::Relation)]
pub struct Relation {
    pub strand: String,
    pub message: String,
    pub seq: i64,
    pub created: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = message::Placed)]
pub struct Placed {
    pub relation: Relation,
    pub message: Message,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = message::Event)]
pub struct Event {
    pub id: String,
    pub message: String,
    pub action: String,
    pub role: Role,
    pub actor: String,
    pub base_version: i64,
    pub payload: Value,
    pub created: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "beat", rename_all = "snake_case")]
#[schema(as = message::Beat)]
pub enum Beat {
    Created {
        message: Placed,
    },
    Delta {
        message: String,
        turn: String,
        role: Role,
        text: String,
    },
    Completed {
        turn: String,
        message: Placed,
    },
}
