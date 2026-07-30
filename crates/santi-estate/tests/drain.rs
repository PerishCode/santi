use santi_estate::{
    CallDraft, DrainDraft, EffectDraft, InboxDraft, NoticeDraft, Opening, Store, StrandDraft,
};
use santi_model::{effect, ingest, message, receipt, turn};

const SUDO: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const FIRST: &str = "2026-07-28T00:00:00.000Z";
const LATER: &str = "2026-07-28T00:01:00.000Z";

#[tokio::test]
async fn opens() {
    let temp = tempfile::tempdir().expect("temp");
    let path = temp.path().join("estate.sqlite");
    let store = Store::bootstrap(&path, SUDO).await.expect("open");
    store.seed("soul_test", FIRST).await.expect("seed");
    let strand = store
        .create_strand(StrandDraft {
            tag: "strand_test",
            soul: "soul_test",
            label: None,
            parent: None,
            fork: None,
            created: FIRST,
        })
        .await
        .expect("strand");
    store
        .accept_inbox(
            InboxDraft {
                tag: "inbox_text",
                strand: &strand.id,
                kind: message::Kind::Text,
                content: &message::Content::text("hello"),
                source: None,
                created: FIRST,
            },
            10,
        )
        .await
        .expect("text");
    let source = ingest::Source::new("job");
    store
        .offer_notice(
            NoticeDraft {
                tag: "inbox_notice_one",
                strand: &strand.id,
                key: "attention_one",
                revision: 1,
                digest: "digest_one",
                content: &message::Content::text("job one"),
                source: &source,
                causes: &["slow".to_string()],
                created: FIRST,
            },
            10,
        )
        .await
        .expect("notice one");
    store
        .offer_notice(
            NoticeDraft {
                tag: "inbox_notice_two",
                strand: &strand.id,
                key: "attention_two",
                revision: 1,
                digest: "digest_two",
                content: &message::Content::text("job two"),
                source: &source,
                causes: &["large".to_string()],
                created: FIRST,
            },
            10,
        )
        .await
        .expect("notice two");

    let opened = store
        .drain_turn(DrainDraft {
            turn: "turn_one",
            strand: &strand.id,
            trigger: turn::Trigger::System,
            source: Some("drain"),
            actor: "santi",
            created: LATER,
        })
        .await
        .expect("drain");
    let Opening::Started(started) = opened else {
        panic!("expected started turn");
    };
    assert_eq!(started.turn.from, 2);
    assert_eq!(started.drained.len(), 2);
    assert_eq!(started.drained[0].text, "hello");
    assert!(started.drained[1].text.contains("items: 2"));
    assert!(started.drained[1].text.contains("job one"));
    assert!(started.drained[1].text.contains("job two"));
    assert!(store.inboxes(&strand.id).await.expect("empty").is_empty());
    assert_eq!(store.messages(&strand.id).await.expect("messages").len(), 2);
    for inbox in ["inbox_text", "inbox_notice_one", "inbox_notice_two"] {
        let receipt = store
            .receipt(inbox)
            .await
            .expect("receipt")
            .expect("status");
        assert_eq!(receipt.state, receipt::State::Driving);
        assert_eq!(receipt.transitions.len(), 2);
        assert_eq!(
            receipt.transitions[1].turn.as_deref(),
            Some(started.turn.id.as_str())
        );
    }

    store
        .offer_notice(
            NoticeDraft {
                tag: "inbox_notice_next",
                strand: &strand.id,
                key: "attention_one",
                revision: 2,
                digest: "digest_next",
                content: &message::Content::text("job next"),
                source: &source,
                causes: &[],
                created: LATER,
            },
            10,
        )
        .await
        .expect("next notice");
    let running = store
        .drain_turn(DrainDraft {
            turn: "turn_ignored",
            strand: &strand.id,
            trigger: turn::Trigger::System,
            source: None,
            actor: "santi",
            created: LATER,
        })
        .await
        .expect("running");
    let Opening::Running(running) = running else {
        panic!("expected running turn");
    };
    assert_eq!(running.id, "turn_one");
    assert_eq!(store.inboxes(&strand.id).await.expect("held").len(), 1);

    store
        .complete_turn("turn_one", 2, LATER)
        .await
        .expect("complete");
    assert_eq!(
        store
            .receipt("inbox_text")
            .await
            .expect("receipt")
            .expect("status")
            .state,
        receipt::State::Completed
    );
    assert!(
        store
            .drain_turn(DrainDraft {
                turn: "turn_one",
                strand: &strand.id,
                trigger: turn::Trigger::System,
                source: None,
                actor: "santi",
                created: LATER,
            })
            .await
            .is_err()
    );
    assert_eq!(store.inboxes(&strand.id).await.expect("rollback").len(), 1);
    assert_eq!(store.messages(&strand.id).await.expect("rollback").len(), 2);

    let opened = store
        .drain_turn(DrainDraft {
            turn: "turn_two",
            strand: &strand.id,
            trigger: turn::Trigger::System,
            source: None,
            actor: "santi",
            created: LATER,
        })
        .await
        .expect("second drain");
    assert!(matches!(opened, Opening::Started(_)));
    let call = store
        .create_call(CallDraft {
            tag: "call_interrupted",
            turn: "turn_two",
            tool: "shell",
            arguments: &serde_json::json!({"command": "sleep 1"}),
            created: LATER,
        })
        .await
        .expect("call");
    let effect = store
        .prepare_effect(EffectDraft {
            tag: "effect_interrupted",
            turn: "turn_two",
            call: Some(&call.id),
            kind: "shell",
            metadata: None,
            created: LATER,
        })
        .await
        .expect("effect");
    store
        .dispatch_effect(&effect.id, LATER)
        .await
        .expect("dispatch");
    store
        .fail_turn("turn_two", "interrupted", LATER)
        .await
        .expect("fail second");
    assert_eq!(
        store
            .effect(&effect.id)
            .await
            .expect("effect")
            .expect("status")
            .effect
            .state,
        effect::State::Unknown
    );
    let idle = store
        .drain_turn(DrainDraft {
            turn: "turn_three",
            strand: &strand.id,
            trigger: turn::Trigger::System,
            source: None,
            actor: "santi",
            created: LATER,
        })
        .await
        .expect("idle");
    assert!(matches!(idle, Opening::Idle));

    drop(store);
    let store = Store::open(path).await.expect("open again");
    assert_eq!(store.messages(&strand.id).await.expect("messages").len(), 3);
    assert_eq!(
        store
            .receipt("inbox_notice_next")
            .await
            .expect("receipt")
            .expect("status")
            .state,
        receipt::State::Failed
    );
}
