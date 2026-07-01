use rusqlite::Row;
use serde_json::Value;

use crate::{
    ActorType, Compact, Message, MessageContent, MessageKind, MessageState, SessionEffect,
    SessionMessage, SessionMessageRef, SoulProfile, Strand, StrandTargetType,
    ThinkingCompletionReason, ThinkingSpan, ThinkingSpanState, ToolCall, ToolResult, Turn,
    TurnStatus, TurnTriggerType, WebhookSubscription,
};

pub(super) fn map_soul_profile_row(row: &Row<'_>) -> rusqlite::Result<SoulProfile> {
    Ok(SoulProfile {
        soul_id: row.get(0)?,
        soul_name: row.get(1)?,
        nickname: row.get(2)?,
        avatar_ref: row.get(3)?,
        avatar_seed: row.get(4)?,
        desc: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

pub(super) fn map_webhook_row(row: &Row<'_>) -> rusqlite::Result<WebhookSubscription> {
    Ok(WebhookSubscription {
        name: row.get(0)?,
        adaptor: row.get(1)?,
        soul_id: row.get(2)?,
        session_strategy: row.get(3)?,
        secret_env: row.get(4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
    })
}

pub(super) fn map_strand_row(row: &Row<'_>) -> rusqlite::Result<Strand> {
    let provider_state: Option<String> = row.get(4)?;
    Ok(Strand {
        id: row.get(0)?,
        soul_id: row.get(1)?,
        external_label: row.get(2)?,
        session_memory: row.get(3)?,
        provider_state: provider_state.and_then(|value| serde_json::from_str(&value).ok()),
        next_seq: row.get(5)?,
        last_seen_session_seq: row.get(6)?,
        parent_strand_id: row.get(7)?,
        fork_point: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
    })
}

pub(super) fn map_message_row(row: &Row<'_>) -> rusqlite::Result<Message> {
    let content_json: String = row.get(4)?;
    let content = serde_json::from_str::<MessageContent>(&content_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(Message {
        id: row.get(0)?,
        actor_type: actor_type_from_db(row.get::<_, String>(1)?.as_str()),
        actor_id: row.get(2)?,
        message_kind: message_kind_from_db(row.get::<_, String>(3)?.as_str()),
        content,
        state: message_state_from_db(row.get::<_, String>(5)?.as_str()),
        version: row.get(6)?,
        deleted_at: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

pub(super) fn map_session_message_row(row: &Row<'_>) -> rusqlite::Result<SessionMessage> {
    let content_json: String = row.get(8)?;
    let content = serde_json::from_str::<MessageContent>(&content_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(8, rusqlite::types::Type::Text, Box::new(error))
    })?;
    let actor_type = actor_type_from_db(row.get::<_, String>(5)?.as_str());
    let message_kind = message_kind_from_db(row.get::<_, String>(7)?.as_str());
    let state = message_state_from_db(row.get::<_, String>(9)?.as_str());
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
    Ok(SessionMessage {
        relation: SessionMessageRef {
            strand_id: row.get(0)?,
            message_id: row.get(1)?,
            strand_seq: row.get(2)?,
            created_at: row.get(3)?,
        },
        message,
        content_text,
    })
}

pub(super) fn map_turn_row(row: &Row<'_>) -> rusqlite::Result<Turn> {
    Ok(Turn {
        id: row.get(0)?,
        strand_id: row.get(1)?,
        trigger_type: turn_trigger_from_db(row.get::<_, String>(2)?.as_str()),
        trigger_ref: row.get(3)?,
        base_strand_seq: row.get(4)?,
        end_strand_seq: row.get(5)?,
        status: turn_status_from_db(row.get::<_, String>(6)?.as_str()),
        error_text: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
        finished_at: row.get(10)?,
    })
}

pub(super) fn map_tool_call_row(row: &Row<'_>) -> rusqlite::Result<ToolCall> {
    let arguments_text: String = row.get(3)?;
    let arguments = serde_json::from_str::<Value>(&arguments_text).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(error))
    })?;
    let provider_item: Option<String> = row.get(4)?;
    Ok(ToolCall {
        id: row.get(0)?,
        turn_id: row.get(1)?,
        tool_name: row.get(2)?,
        arguments,
        provider_item: provider_item.and_then(|value| serde_json::from_str(&value).ok()),
        item_id: row.get(5)?,
        response_id: row.get(6)?,
        created_at: row.get(7)?,
    })
}

pub(super) fn map_tool_result_row(row: &Row<'_>) -> rusqlite::Result<ToolResult> {
    let output_text: Option<String> = row.get(2)?;
    Ok(ToolResult {
        id: row.get(0)?,
        tool_call_id: row.get(1)?,
        output: output_text.and_then(|value| serde_json::from_str(&value).ok()),
        error_text: row.get(3)?,
        created_at: row.get(4)?,
    })
}

pub(super) fn map_thinking_span_row(row: &Row<'_>) -> rusqlite::Result<ThinkingSpan> {
    Ok(ThinkingSpan {
        id: row.get(0)?,
        turn_id: row.get(1)?,
        provider_response_id: row.get(2)?,
        state: thinking_state_from_db(row.get::<_, String>(3)?.as_str()),
        summary: row.get(4)?,
        completion_reason: row
            .get::<_, Option<String>>(5)?
            .as_deref()
            .map(completion_reason_from_db),
        error_text: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
        finished_at: row.get(9)?,
    })
}

pub(super) fn map_compact_row(row: &Row<'_>) -> rusqlite::Result<Compact> {
    Ok(Compact {
        id: row.get(0)?,
        strand_id: row.get(1)?,
        summary: row.get(2)?,
        start_message_id: row.get(3)?,
        end_message_id: row.get(4)?,
    })
}

pub(super) fn map_session_effect_row(row: &Row<'_>) -> rusqlite::Result<SessionEffect> {
    Ok(SessionEffect {
        id: row.get(0)?,
        strand_id: row.get(1)?,
        effect_type: row.get(2)?,
        idempotency_key: row.get(3)?,
        status: row.get(4)?,
        source_hook_id: row.get(5)?,
        source_turn_id: row.get(6)?,
        result_ref: row.get(7)?,
        error_text: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
    })
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

pub(super) fn actor_type_db(value: &ActorType) -> &'static str {
    match value {
        ActorType::Soul => "soul",
        ActorType::System => "system",
    }
}

fn actor_type_from_db(value: &str) -> ActorType {
    match value {
        "soul" => ActorType::Soul,
        _ => ActorType::System,
    }
}

pub(super) fn message_state_db(value: &MessageState) -> &'static str {
    match value {
        MessageState::Pending => "pending",
        MessageState::Fixed => "fixed",
        MessageState::Aborted => "aborted",
    }
}

pub(super) fn message_kind_db(value: &MessageKind) -> &'static str {
    match value {
        MessageKind::Text => "text",
        MessageKind::SantiSystem => "santi_system",
    }
}

fn message_kind_from_db(value: &str) -> MessageKind {
    match value {
        "text" => MessageKind::Text,
        "santi_system" => MessageKind::SantiSystem,
        _ => MessageKind::Text,
    }
}

fn message_state_from_db(value: &str) -> MessageState {
    match value {
        "pending" => MessageState::Pending,
        "fixed" => MessageState::Fixed,
        "aborted" => MessageState::Aborted,
        _ => MessageState::Fixed,
    }
}

fn turn_trigger_from_db(value: &str) -> TurnTriggerType {
    match value {
        "session_send" => TurnTriggerType::SessionSend,
        "system" => TurnTriggerType::System,
        _ => TurnTriggerType::System,
    }
}

fn turn_status_from_db(value: &str) -> TurnStatus {
    match value {
        "running" => TurnStatus::Running,
        "completed" => TurnStatus::Completed,
        "failed" => TurnStatus::Failed,
        _ => TurnStatus::Failed,
    }
}

pub(super) fn thinking_span_state_db(value: &ThinkingSpanState) -> &'static str {
    match value {
        ThinkingSpanState::Running => "running",
        ThinkingSpanState::Completed => "completed",
        ThinkingSpanState::Failed => "failed",
    }
}

pub(super) fn thinking_completion_reason_db(value: &ThinkingCompletionReason) -> &'static str {
    match value {
        ThinkingCompletionReason::FirstTextDelta => "first_text_delta",
        ThinkingCompletionReason::ToolCallRequested => "tool_call_requested",
        ThinkingCompletionReason::ProviderCompleted => "provider_completed",
    }
}

fn completion_reason_from_db(value: &str) -> ThinkingCompletionReason {
    match value {
        "first_text_delta" => ThinkingCompletionReason::FirstTextDelta,
        "tool_call_requested" => ThinkingCompletionReason::ToolCallRequested,
        "provider_completed" => ThinkingCompletionReason::ProviderCompleted,
        _ => ThinkingCompletionReason::ProviderCompleted,
    }
}

fn thinking_state_from_db(value: &str) -> ThinkingSpanState {
    match value {
        "running" => ThinkingSpanState::Running,
        "completed" => ThinkingSpanState::Completed,
        "failed" => ThinkingSpanState::Failed,
        _ => ThinkingSpanState::Failed,
    }
}

pub(super) fn entry_type_db(value: &StrandTargetType) -> &'static str {
    match value {
        StrandTargetType::Message => "message",
        StrandTargetType::Compact => "compact",
        StrandTargetType::Thinking => "thinking",
        StrandTargetType::ToolCall => "tool_call",
        StrandTargetType::ToolResult => "tool_result",
    }
}
