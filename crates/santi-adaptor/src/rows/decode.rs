use rusqlite::Row;
use santi_model::{
    ActorType, Compact, DownstreamCredential, EffectState, Message, MessageContent, MessageEvent,
    MessageKind, MessageState, Soul, Strand, StrandEffect, StrandMessage, StrandMessageRef,
    ThinkingCompletionReason, ThinkingSpan, ThinkingSpanState, ToolCall, ToolResult, Turn,
    TurnStatus, TurnTriggerType, WebhookSubscription,
};
use serde_json::Value;

use super::*;

impl Decode for Soul {
    fn decode(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            created: row.get(1)?,
            updated: row.get(2)?,
        })
    }
}

impl Decode for DownstreamCredential {
    fn decode(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            prefix: row.get(1)?,
            digest: row.get(2)?,
            created: row.get(3)?,
            updated: row.get(4)?,
        })
    }
}

impl Decode for WebhookSubscription {
    fn decode(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            name: row.get(0)?,
            adaptor: row.get(1)?,
            soul: row.get(2)?,
            strategy: row.get(3)?,
            credential: row.get(4)?,
            created: row.get(5)?,
            updated: row.get(6)?,
        })
    }
}

impl Decode for Strand {
    fn decode(row: &Row<'_>) -> rusqlite::Result<Self> {
        let state: Option<String> = row.get(4)?;
        Ok(Self {
            id: row.get(0)?,
            soul: row.get(1)?,
            label: row.get(2)?,
            memory: row.get(3)?,
            state: state.and_then(|value| serde_json::from_str(&value).ok()),
            next: row.get(5)?,
            seen: row.get(6)?,
            parent: row.get(7)?,
            fork: row.get(8)?,
            created: row.get(9)?,
            updated: row.get(10)?,
        })
    }
}

impl Decode for Message {
    fn decode(row: &Row<'_>) -> rusqlite::Result<Self> {
        let content_json: String = row.get(4)?;
        let content = serde_json::from_str::<MessageContent>(&content_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                4,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
        Ok(Self {
            id: row.get(0)?,
            role: ActorType::decode(row.get::<_, String>(1)?.as_str()),
            actor: row.get(2)?,
            kind: MessageKind::decode(row.get::<_, String>(3)?.as_str()),
            content,
            state: MessageState::decode(row.get::<_, String>(5)?.as_str()),
            version: row.get(6)?,
            deleted: row.get(7)?,
            created: row.get(8)?,
            updated: row.get(9)?,
        })
    }
}

impl Decode for MessageEvent {
    fn decode(row: &Row<'_>) -> rusqlite::Result<Self> {
        let payload_json: String = row.get(6)?;
        let payload = serde_json::from_str::<Value>(&payload_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                6,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
        Ok(Self {
            id: row.get(0)?,
            message: row.get(1)?,
            action: row.get(2)?,
            role: ActorType::decode(row.get::<_, String>(3)?.as_str()),
            actor: row.get(4)?,
            base_version: row.get(5)?,
            payload,
            created: row.get(7)?,
        })
    }
}

impl Decode for StrandMessage {
    fn decode(row: &Row<'_>) -> rusqlite::Result<Self> {
        let content_json: String = row.get(8)?;
        let content = serde_json::from_str::<MessageContent>(&content_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                8,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
        let role = ActorType::decode(row.get::<_, String>(5)?.as_str());
        let kind = MessageKind::decode(row.get::<_, String>(7)?.as_str());
        let state = MessageState::decode(row.get::<_, String>(9)?.as_str());
        let message = Message {
            id: row.get(4)?,
            role,
            actor: row.get(6)?,
            kind,
            content,
            state,
            version: row.get(10)?,
            deleted: row.get(11)?,
            created: row.get(12)?,
            updated: row.get(13)?,
        };
        let text = message.content.rendered();
        Ok(Self {
            relation: StrandMessageRef {
                strand: row.get(0)?,
                message: row.get(1)?,
                seq: row.get(2)?,
                created: row.get(3)?,
            },
            message,
            text,
        })
    }
}

impl Decode for Turn {
    fn decode(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            strand: row.get(1)?,
            trigger: TurnTriggerType::decode(row.get::<_, String>(2)?.as_str()),
            source: row.get(3)?,
            from: row.get(4)?,
            to: row.get(5)?,
            status: TurnStatus::decode(row.get::<_, String>(6)?.as_str()),
            error: row.get(7)?,
            created: row.get(8)?,
            updated: row.get(9)?,
            finished: row.get(10)?,
        })
    }
}

impl Decode for ToolCall {
    fn decode(row: &Row<'_>) -> rusqlite::Result<Self> {
        let arguments_text: String = row.get(3)?;
        let arguments = serde_json::from_str::<Value>(&arguments_text).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                3,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
        Ok(Self {
            id: row.get(0)?,
            turn: row.get(1)?,
            tool: row.get(2)?,
            arguments,
            created: row.get(4)?,
        })
    }
}

impl Decode for ToolResult {
    fn decode(row: &Row<'_>) -> rusqlite::Result<Self> {
        let output_text: Option<String> = row.get(2)?;
        Ok(Self {
            id: row.get(0)?,
            call: row.get(1)?,
            output: output_text.and_then(|value| serde_json::from_str(&value).ok()),
            error: row.get(3)?,
            created: row.get(4)?,
        })
    }
}

impl Decode for ThinkingSpan {
    fn decode(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            turn: row.get(1)?,
            response: row.get(2)?,
            state: ThinkingSpanState::decode(row.get::<_, String>(3)?.as_str()),
            summary: row.get(4)?,
            completion_reason: row
                .get::<_, Option<String>>(5)?
                .as_deref()
                .map(ThinkingCompletionReason::decode),
            error: row.get(6)?,
            created: row.get(7)?,
            updated: row.get(8)?,
            finished: row.get(9)?,
        })
    }
}

impl Decode for Compact {
    fn decode(row: &Row<'_>) -> rusqlite::Result<Self> {
        let metadata_json: Option<String> = row.get(6)?;
        Ok(Self {
            id: row.get(0)?,
            strand: row.get(1)?,
            summary: row.get(2)?,
            first: row.get(3)?,
            last: row.get(4)?,
            created: row.get(5)?,
            metadata: metadata_json.and_then(|value| serde_json::from_str(&value).ok()),
        })
    }
}

impl Decode for StrandEffect {
    fn decode(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            strand: row.get(1)?,
            turn: row.get(2)?,
            call: row.get(3)?,
            kind: row.get(4)?,
            state: EffectState::decode(&row.get::<_, String>(5)?),
            result: row.get(6)?,
            error: row.get(7)?,
            created: row.get(8)?,
            updated: row.get(9)?,
            dispatched: row.get(10)?,
            settled: row.get(11)?,
        })
    }
}
