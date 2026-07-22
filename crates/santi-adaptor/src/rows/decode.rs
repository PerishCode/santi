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
            created_at: row.get(1)?,
            updated_at: row.get(2)?,
        })
    }
}

impl Decode for DownstreamCredential {
    fn decode(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            label_prefix: row.get(1)?,
            credential_env: row.get(2)?,
            created_at: row.get(3)?,
            updated_at: row.get(4)?,
        })
    }
}

impl Decode for WebhookSubscription {
    fn decode(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            name: row.get(0)?,
            adaptor: row.get(1)?,
            soul_id: row.get(2)?,
            strand_strategy: row.get(3)?,
            secret_env: row.get(4)?,
            created_at: row.get(5)?,
            updated_at: row.get(6)?,
        })
    }
}

impl Decode for Strand {
    fn decode(row: &Row<'_>) -> rusqlite::Result<Self> {
        let provider_state: Option<String> = row.get(4)?;
        Ok(Self {
            id: row.get(0)?,
            soul_id: row.get(1)?,
            external_label: row.get(2)?,
            strand_memory: row.get(3)?,
            provider_state: provider_state.and_then(|value| serde_json::from_str(&value).ok()),
            next_seq: row.get(5)?,
            last_seen_strand_seq: row.get(6)?,
            parent_strand_id: row.get(7)?,
            fork_point: row.get(8)?,
            created_at: row.get(9)?,
            updated_at: row.get(10)?,
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
            actor_type: ActorType::decode(row.get::<_, String>(1)?.as_str()),
            actor_id: row.get(2)?,
            message_kind: MessageKind::decode(row.get::<_, String>(3)?.as_str()),
            content,
            state: MessageState::decode(row.get::<_, String>(5)?.as_str()),
            version: row.get(6)?,
            deleted_at: row.get(7)?,
            created_at: row.get(8)?,
            updated_at: row.get(9)?,
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
            message_id: row.get(1)?,
            action: row.get(2)?,
            actor_type: ActorType::decode(row.get::<_, String>(3)?.as_str()),
            actor_id: row.get(4)?,
            base_version: row.get(5)?,
            payload,
            created_at: row.get(7)?,
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
        let actor_type = ActorType::decode(row.get::<_, String>(5)?.as_str());
        let message_kind = MessageKind::decode(row.get::<_, String>(7)?.as_str());
        let state = MessageState::decode(row.get::<_, String>(9)?.as_str());
        let message = Message {
            id: row.get(4)?,
            actor_type,
            actor_id: row.get(6)?,
            message_kind,
            content,
            state,
            version: row.get(10)?,
            deleted_at: row.get(11)?,
            created_at: row.get(12)?,
            updated_at: row.get(13)?,
        };
        let content_text = message.content.content_text();
        Ok(Self {
            relation: StrandMessageRef {
                strand_id: row.get(0)?,
                message_id: row.get(1)?,
                strand_seq: row.get(2)?,
                created_at: row.get(3)?,
            },
            message,
            content_text,
        })
    }
}

impl Decode for Turn {
    fn decode(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            strand_id: row.get(1)?,
            trigger_type: TurnTriggerType::decode(row.get::<_, String>(2)?.as_str()),
            trigger_ref: row.get(3)?,
            base_strand_seq: row.get(4)?,
            end_strand_seq: row.get(5)?,
            status: TurnStatus::decode(row.get::<_, String>(6)?.as_str()),
            error_text: row.get(7)?,
            created_at: row.get(8)?,
            updated_at: row.get(9)?,
            finished_at: row.get(10)?,
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
            turn_id: row.get(1)?,
            tool_name: row.get(2)?,
            arguments,
            created_at: row.get(4)?,
        })
    }
}

impl Decode for ToolResult {
    fn decode(row: &Row<'_>) -> rusqlite::Result<Self> {
        let output_text: Option<String> = row.get(2)?;
        Ok(Self {
            id: row.get(0)?,
            tool_call_id: row.get(1)?,
            output: output_text.and_then(|value| serde_json::from_str(&value).ok()),
            error_text: row.get(3)?,
            created_at: row.get(4)?,
        })
    }
}

impl Decode for ThinkingSpan {
    fn decode(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            turn_id: row.get(1)?,
            provider_response_id: row.get(2)?,
            state: ThinkingSpanState::decode(row.get::<_, String>(3)?.as_str()),
            summary: row.get(4)?,
            completion_reason: row
                .get::<_, Option<String>>(5)?
                .as_deref()
                .map(ThinkingCompletionReason::decode),
            error_text: row.get(6)?,
            created_at: row.get(7)?,
            updated_at: row.get(8)?,
            finished_at: row.get(9)?,
        })
    }
}

impl Decode for Compact {
    fn decode(row: &Row<'_>) -> rusqlite::Result<Self> {
        let metadata_json: Option<String> = row.get(6)?;
        Ok(Self {
            id: row.get(0)?,
            strand_id: row.get(1)?,
            summary: row.get(2)?,
            start_message_id: row.get(3)?,
            end_message_id: row.get(4)?,
            created_at: row.get(5)?,
            metadata: metadata_json.and_then(|value| serde_json::from_str(&value).ok()),
        })
    }
}

impl Decode for StrandEffect {
    fn decode(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            strand_id: row.get(1)?,
            turn_id: row.get(2)?,
            tool_call_id: row.get(3)?,
            effect_type: row.get(4)?,
            state: EffectState::decode(&row.get::<_, String>(5)?),
            result_ref: row.get(6)?,
            error_text: row.get(7)?,
            created_at: row.get(8)?,
            updated_at: row.get(9)?,
            dispatched_at: row.get(10)?,
            settled_at: row.get(11)?,
        })
    }
}
