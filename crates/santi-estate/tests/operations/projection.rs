use santi_estate::{
    CallDraft, DrainDraft, EffectDraft, InboxDraft, Opening, ReplyDraft, Store, StrandDraft,
    ThinkingDraft,
};
use santi_model::{message, strand, thinking, turn};

const FIRST: &str = "2026-07-28T00:00:00.000Z";
const LATER: &str = "2026-07-28T00:01:00.000Z";

#[tokio::test]
async fn timeline() {
    let temp = tempfile::tempdir().expect("temp");
    let path = temp.path().join("estate.sqlite");
    let store = Store::open(&path).await.expect("open");
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
                tag: "inbox_test",
                strand: &strand.id,
                kind: message::Kind::Text,
                content: &message::Content::text("hello"),
                source: None,
                created: FIRST,
            },
            10,
        )
        .await
        .expect("inbox");
    let Opening::Started(opened) = store
        .drain_turn(DrainDraft {
            turn: "turn_test",
            strand: &strand.id,
            trigger: turn::Trigger::System,
            source: None,
            actor: "santi",
            created: LATER,
        })
        .await
        .expect("drain")
    else {
        panic!("expected started turn");
    };
    let span = store
        .create_thinking(ThinkingDraft {
            tag: "thinking_test",
            turn: &opened.turn.id,
            response: Some("response_test"),
            created: LATER,
        })
        .await
        .expect("thinking");
    store
        .complete_thinking(&span.id, thinking::Reason::Called, LATER)
        .await
        .expect("finish thinking");
    let call = store
        .create_call(CallDraft {
            tag: "call_test",
            turn: &opened.turn.id,
            tool: "shell",
            arguments: &serde_json::json!({"command": "true"}),
            created: LATER,
        })
        .await
        .expect("call");
    store
        .prepare_effect(EffectDraft {
            tag: "effect_test",
            turn: &opened.turn.id,
            call: Some(&call.id),
            kind: "shell",
            metadata: None,
            created: LATER,
        })
        .await
        .expect("effect");
    store
        .create_reply(ReplyDraft {
            tag: "result_test",
            call: &call.id,
            output: Some(&serde_json::json!({"stdout": "done"})),
            error: None,
            created: LATER,
        })
        .await
        .expect("reply");

    assert_eq!(store.events(&strand.id).await.expect("events").len(), 1);
    assert_eq!(
        store.thinkings(&strand.id).await.expect("thinking").len(),
        1
    );
    assert_eq!(
        store.thought(&opened.turn.id).await.expect("thought").len(),
        1
    );
    assert_eq!(store.calls(&strand.id).await.expect("calls").len(), 1);
    assert_eq!(
        store.called(&opened.turn.id).await.expect("called").len(),
        1
    );
    assert_eq!(store.results(&strand.id).await.expect("results").len(), 1);
    assert_eq!(
        store.replied(&opened.turn.id).await.expect("replied").len(),
        1
    );
    assert_eq!(store.effects(&strand.id).await.expect("effects").len(), 1);
    let entries = store.entries(&strand.id).await.expect("entries");
    assert_eq!(entries.len(), 4);
    assert_eq!(entries[0].kind, strand::Target::Message);
    assert_eq!(entries[1].kind, strand::Target::Thinking);
    assert_eq!(entries[2].kind, strand::Target::ToolCall);
    assert_eq!(entries[3].kind, strand::Target::ToolResult);
    assert_eq!(entries[3].seq, 4);

    let snapshot = store
        .snapshot(&strand.id)
        .await
        .expect("snapshot")
        .expect("held");
    assert_eq!(snapshot.messages.len(), 1);
    assert_eq!(snapshot.events.len(), 1);
    assert_eq!(snapshot.turns.len(), 1);
    assert_eq!(snapshot.thinking.len(), 1);
    assert_eq!(snapshot.calls.len(), 1);
    assert_eq!(snapshot.results.len(), 1);
    assert_eq!(snapshot.effects.len(), 1);
    assert!(snapshot.compacts.is_empty());
    assert!(snapshot.errors.is_empty());
    assert!(store.snapshot("missing").await.expect("missing").is_none());

    drop(store);
    let store = Store::open(path).await.expect("open again");
    let snapshot = store
        .snapshot(&strand.id)
        .await
        .expect("snapshot again")
        .expect("held");
    assert_eq!(snapshot.events[0].action, "insert");
    assert_eq!(snapshot.calls[0].id, "call_test");
    assert_eq!(snapshot.results[0].id, "result_test");
}
