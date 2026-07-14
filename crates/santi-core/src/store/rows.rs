use rusqlite::Row;
use serde_json::Value;

use crate::{
    ActorType, Compact, EffectState, EffectTransitionReason, Message, MessageContent, MessageEvent,
    MessageKind, MessageState, Soul, Strand, StrandEffect, StrandMessage, StrandMessageRef,
    StrandTargetType, ThinkingCompletionReason, ThinkingSpan, ThinkingSpanState, ToolCall,
    ToolResult, Turn, TurnStatus, TurnTriggerType, WebhookSubscription,
};

pub(super) trait Decode: Sized {
    fn decode(row: &Row<'_>) -> rusqlite::Result<Self>;
}

impl Decode for Soul {
    fn decode(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            created_at: row.get(1)?,
            updated_at: row.get(2)?,
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

impl EffectState {
    pub(super) fn encode(&self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::Dispatching => "dispatching",
            Self::Unknown => "unknown",
            Self::Confirmed => "confirmed",
            Self::NotDispatched => "not_dispatched",
            Self::ResolvedApplied => "resolved_applied",
            Self::ResolvedNotApplied => "resolved_not_applied",
        }
    }

    pub(super) fn decode(value: &str) -> Self {
        match value {
            "prepared" => Self::Prepared,
            "dispatching" => Self::Dispatching,
            "confirmed" => Self::Confirmed,
            "not_dispatched" => Self::NotDispatched,
            "resolved_applied" => Self::ResolvedApplied,
            "resolved_not_applied" => Self::ResolvedNotApplied,
            _ => Self::Unknown,
        }
    }
}

impl EffectTransitionReason {
    pub(super) fn encode(&self) -> &'static str {
        match self {
            Self::IntentPersisted => "intent_persisted",
            Self::DispatchWindowOpened => "dispatch_window_opened",
            Self::ResultPersisted => "result_persisted",
            Self::DispatchRejected => "dispatch_rejected",
            Self::RestartBeforeDispatch => "restart_before_dispatch",
            Self::RestartDuringDispatch => "restart_during_dispatch",
            Self::TurnFailedBeforeDispatch => "turn_failed_before_dispatch",
            Self::TurnFailedDuringDispatch => "turn_failed_during_dispatch",
            Self::ResultCaptureFailed => "result_capture_failed",
            Self::OperatorResolvedApplied => "operator_resolved_applied",
            Self::OperatorResolvedNotApplied => "operator_resolved_not_applied",
            Self::LegacyImport => "legacy_import",
        }
    }

    pub(super) fn decode(value: &str) -> Self {
        match value {
            "intent_persisted" => Self::IntentPersisted,
            "dispatch_window_opened" => Self::DispatchWindowOpened,
            "result_persisted" => Self::ResultPersisted,
            "dispatch_rejected" => Self::DispatchRejected,
            "restart_before_dispatch" => Self::RestartBeforeDispatch,
            "restart_during_dispatch" => Self::RestartDuringDispatch,
            "turn_failed_before_dispatch" => Self::TurnFailedBeforeDispatch,
            "turn_failed_during_dispatch" => Self::TurnFailedDuringDispatch,
            "result_capture_failed" => Self::ResultCaptureFailed,
            "operator_resolved_applied" => Self::OperatorResolvedApplied,
            "operator_resolved_not_applied" => Self::OperatorResolvedNotApplied,
            _ => Self::LegacyImport,
        }
    }
}

pub(super) fn collect_rows<T>(
    rows: impl Iterator<Item = rusqlite::Result<T>>,
) -> Result<Vec<T>, String> {
    let mut items = Vec::new();
    for row in rows {
        items.push(row.map_err(|error| error.to_string())?);
    }
    Ok(items)
}

impl ActorType {
    pub(super) fn encode(&self) -> &'static str {
        match self {
            Self::Soul => "soul",
            Self::System => "system",
        }
    }

    fn decode(value: &str) -> Self {
        match value {
            "soul" => Self::Soul,
            _ => Self::System,
        }
    }
}

impl MessageState {
    pub(super) fn encode(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Fixed => "fixed",
            Self::Aborted => "aborted",
        }
    }

    fn decode(value: &str) -> Self {
        match value {
            "pending" => Self::Pending,
            "fixed" => Self::Fixed,
            "aborted" => Self::Aborted,
            _ => Self::Fixed,
        }
    }
}

impl MessageKind {
    pub(super) fn encode(&self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::SantiSystem => "santi_system",
        }
    }

    pub(super) fn decode(value: &str) -> Self {
        match value {
            "text" => Self::Text,
            "santi_system" => Self::SantiSystem,
            _ => Self::Text,
        }
    }
}

impl TurnTriggerType {
    fn decode(value: &str) -> Self {
        match value {
            "strand_send" => Self::StrandSend,
            "system" => Self::System,
            _ => Self::System,
        }
    }
}

impl TurnStatus {
    fn decode(value: &str) -> Self {
        match value {
            "running" => Self::Running,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            _ => Self::Failed,
        }
    }
}

impl ThinkingSpanState {
    pub(super) fn encode(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }

    fn decode(value: &str) -> Self {
        match value {
            "running" => Self::Running,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            _ => Self::Failed,
        }
    }
}

impl ThinkingCompletionReason {
    pub(super) fn encode(&self) -> &'static str {
        match self {
            Self::FirstTextDelta => "first_text_delta",
            Self::ToolCallRequested => "tool_call_requested",
            Self::ProviderCompleted => "provider_completed",
        }
    }

    fn decode(value: &str) -> Self {
        match value {
            "first_text_delta" => Self::FirstTextDelta,
            "tool_call_requested" => Self::ToolCallRequested,
            "provider_completed" => Self::ProviderCompleted,
            _ => Self::ProviderCompleted,
        }
    }
}

impl StrandTargetType {
    pub(super) fn encode(&self) -> &'static str {
        match self {
            Self::Message => "message",
            Self::Compact => "compact",
            Self::Thinking => "thinking",
            Self::ToolCall => "tool_call",
            Self::ToolResult => "tool_result",
        }
    }
}
