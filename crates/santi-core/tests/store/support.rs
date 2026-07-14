pub(crate) use rusqlite::Connection;
pub(crate) use santi_core::{
    ActorType, Completion, Draft, EffectResolutionOutcome, EffectState, EffectTransitionReason,
    IngestOutcome, Invocation, MessageContent, MessageIntake, MessageKind, MessageState,
    ProviderItem, ReceiptState, SantiStore, ThinkingCompletionReason, ToolCallProvenance,
};
pub(crate) use serde_json::json;

pub(crate) fn assert_text(item: &ProviderItem, role: &str, content: &str) {
    match item {
        ProviderItem::Message {
            role: actual_role,
            content: actual_content,
        } => {
            assert_eq!(actual_role, role);
            assert_eq!(actual_content, content);
        }
        other => panic!("expected text item, got {other:?}"),
    }
}

pub(crate) struct Line<'a> {
    pub store: &'a SantiStore,
    pub strand: &'a str,
    pub actor: ActorType,
    pub text: &'a str,
    pub intake: MessageIntake,
}

pub(crate) fn append_timeline_message(line: Line<'_>) {
    match line.intake {
        MessageIntake::Request => {
            line.store
                .enqueue_inbox(
                    line.strand,
                    MessageKind::Text,
                    MessageContent::text(line.text),
                )
                .expect("enqueue inbox");
        }
        MessageIntake::Record => {
            let actor_id = match line.actor {
                ActorType::Soul => line.store.default_soul_id(),
                ActorType::System => line.store.system_actor_id(),
            };
            line.store
                .append_message(Draft {
                    strand: line.strand,
                    actor: line.actor,
                    id: actor_id,
                    content: MessageContent::text(line.text),
                    state: MessageState::Fixed,
                    intake: line.intake,
                })
                .expect("append message");
        }
    }
}
