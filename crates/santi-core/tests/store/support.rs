pub(crate) use rusqlite::Connection;
pub(crate) use santi_core::{
    ActorType, EffectResolutionOutcome, EffectState, EffectTransitionReason, IngestOutcome,
    MessageContent, MessageIntake, MessageKind, MessageState, ProviderItem, ReceiptState,
    SantiStore, ThinkingCompletionReason, ToolCallProvenance,
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

pub(crate) fn append_timeline_message(
    store: &SantiStore,
    strand_id: &str,
    actor_type: ActorType,
    text: &str,
    intake: MessageIntake,
) {
    match intake {
        MessageIntake::Request => {
            store
                .enqueue_inbox(strand_id, MessageKind::Text, MessageContent::text(text))
                .expect("enqueue inbox");
        }
        MessageIntake::Record => {
            let actor_id = match actor_type {
                ActorType::Soul => store.default_soul_id(),
                ActorType::System => store.system_actor_id(),
            };
            store
                .append_message(
                    strand_id,
                    actor_type,
                    actor_id,
                    MessageContent::text(text),
                    MessageState::Fixed,
                    intake,
                )
                .expect("append message");
        }
    }
}
