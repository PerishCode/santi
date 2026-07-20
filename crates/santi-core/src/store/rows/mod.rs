use rusqlite::Row;

use crate::{
    ActorType, EffectState, EffectTransitionReason, MessageKind, MessageState, StrandTargetType,
    ThinkingCompletionReason, ThinkingSpanState, TurnStatus, TurnTriggerType,
};

mod decode;

pub(super) trait Decode: Sized {
    fn decode(row: &Row<'_>) -> rusqlite::Result<Self>;
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
