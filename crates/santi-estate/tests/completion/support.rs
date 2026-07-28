use santi_error::{Draft, Scope, Source};
use santi_estate::{EffectDraft, InboxDraft, Store, StrandDraft, TurnDraft};
use santi_model::{message, receipt, turn};

pub(super) const FIRST: &str = "2026-07-28T00:00:00.000Z";

pub(super) async fn create_strand(store: &Store, tag: &str, label: Option<&str>) {
    store
        .create_strand(StrandDraft {
            tag,
            soul: "soul_test",
            label,
            parent: None,
            fork: None,
            created: FIRST,
        })
        .await
        .expect("strand");
}

pub(super) async fn driven(store: &Store, suffix: &str) {
    let strand = format!("strand_{suffix}");
    let inbox = format!("inbox_{suffix}");
    let turn = format!("turn_{suffix}");
    let effect = format!("effect_{suffix}");
    let content = message::Content::text("drive");
    store
        .accept_inbox(
            InboxDraft {
                tag: &inbox,
                strand: &strand,
                kind: message::Kind::Text,
                content: &content,
                source: None,
                created: FIRST,
            },
            10,
        )
        .await
        .expect("inbox");
    create_turn(store, &turn, &strand).await;
    store
        .advance_receipt(
            &inbox,
            receipt::State::Driving,
            Some(&turn),
            None,
            None,
            FIRST,
        )
        .await
        .expect("receipt");
    store
        .prepare_effect(EffectDraft {
            tag: &effect,
            turn: &turn,
            call: None,
            kind: "test",
            metadata: None,
            created: FIRST,
        })
        .await
        .expect("effect");
}

pub(super) async fn create_turn(store: &Store, tag: &str, strand: &str) {
    store
        .create_turn(TurnDraft {
            tag,
            strand,
            trigger: turn::Trigger::System,
            source: None,
            from: 0,
            created: FIRST,
        })
        .await
        .expect("turn");
}

pub(super) fn incident(descriptor: santi_error::Descriptor, scope: &Scope, message: &str) -> Draft {
    Draft {
        key: descriptor.key(&scope.kind, &scope.id),
        descriptor,
        scope: scope.clone(),
        source: Source::new("santi-core", "test"),
        message: message.to_string(),
        context: serde_json::json!({"message": message}),
    }
}
