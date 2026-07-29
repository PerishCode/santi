use super::{Environment, Service};
use crate::{message, stream};

pub(super) async fn warn(service: &Service, event: Environment) -> Result<(), String> {
    let content = message::Content::text(
        [
            "<system_message>".to_string(),
            "kind: environment_unresolved".to_string(),
            "scope: strand_local".to_string(),
            "wake: false".to_string(),
            "obligation: false".to_string(),
            format!("trigger_turn_id: {}", event.address.turn),
            format!("declaration_scope: {}", event.issue.scope),
            format!("name: {}", event.issue.name),
            format!("reference: env://{}", event.issue.reference),
            "summary: An environment reference was unresolved. The shell received the original env:// value so the soul can diagnose and report it.".to_string(),
            "</system_message>".to_string(),
        ]
        .join("\n"),
    );
    let message = service
        .store
        .place(santi_estate::MessageDraft {
            tag: &crate::tag("msg"),
            strand: &event.address.strand,
            actor: message::Role::System,
            actor_id: crate::SYSTEM,
            kind: message::Kind::SantiSystem,
            content: &content,
            state: message::State::Fixed,
            request: false,
            created: &crate::now(),
        })
        .await?;
    service.publish(
        &event.address.strand,
        stream::Payload::Message(crate::message::Beat::Created { message }),
    );
    Ok(())
}
