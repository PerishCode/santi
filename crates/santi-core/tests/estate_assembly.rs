use santi_core::{Item, provider_input, provider_preview};
use santi_estate::{
    CallDraft, CompactDraft, MessageDraft, ReplyDraft, Store, StrandDraft, ThinkingDraft, TurnDraft,
};
use santi_model::{message, thinking, turn};

const FIRST: &str = "2026-07-28T00:00:00.000Z";
const LATER: &str = "2026-07-28T00:01:00.000Z";

#[tokio::test]
async fn assembles_ordered_estate() {
    let temp = tempfile::tempdir().expect("temp");
    let store = Store::open(temp.path().join("estate.sqlite"))
        .await
        .expect("open");
    store.seed("soul_test", FIRST).await.expect("seed");
    store
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
        .place(MessageDraft {
            tag: "message_first",
            strand: "strand_test",
            actor: message::Role::System,
            actor_id: "santi",
            kind: message::Kind::Text,
            content: &message::Content::text("run"),
            state: message::State::Fixed,
            request: true,
            created: FIRST,
        })
        .await
        .expect("first");
    store
        .create_turn(TurnDraft {
            tag: "turn_test",
            strand: "strand_test",
            trigger: turn::Trigger::System,
            source: None,
            from: 1,
            created: FIRST,
        })
        .await
        .expect("turn");
    store
        .create_thinking(ThinkingDraft {
            tag: "thinking_test",
            turn: "turn_test",
            response: Some("response_test"),
            created: FIRST,
        })
        .await
        .expect("thinking");
    store
        .update_thinking("thinking_test", None, Some("reasoned"), LATER)
        .await
        .expect("summarize");
    store
        .complete_thinking("thinking_test", thinking::Reason::Called, LATER)
        .await
        .expect("complete thinking");
    store
        .create_call(CallDraft {
            tag: "call_test",
            turn: "turn_test",
            tool: "shell",
            arguments: &serde_json::json!({"command": "true"}),
            created: LATER,
        })
        .await
        .expect("call");
    store
        .create_reply(ReplyDraft {
            tag: "result_test",
            call: "call_test",
            output: Some(&serde_json::json!({"stdout": "done"})),
            error: None,
            created: LATER,
        })
        .await
        .expect("reply");
    store
        .place(MessageDraft {
            tag: "message_last",
            strand: "strand_test",
            actor: message::Role::Soul,
            actor_id: "soul_test",
            kind: message::Kind::Text,
            content: &message::Content::text("done"),
            state: message::State::Fixed,
            request: false,
            created: LATER,
        })
        .await
        .expect("last");

    let input = provider_input(&store, "strand_test")
        .await
        .expect("assembly");
    assert_eq!(input.len(), 5);
    assert_message(&input[0], "user", "run");
    let Item::Reasoning { id, content } = &input[1] else {
        panic!("expected reasoning");
    };
    assert_eq!(id.as_deref(), Some("response_test"));
    assert_eq!(content, "reasoned");
    let Item::Call {
        call,
        raw,
        item,
        mark,
        ..
    } = &input[2]
    else {
        panic!("expected call");
    };
    assert_eq!(call, "call_test");
    assert_eq!(raw, r#"{"command":"true"}"#);
    assert!(item.is_none());
    assert!(mark.is_none());
    let Item::Output { call, output } = &input[3] else {
        panic!("expected output");
    };
    assert_eq!(call, "call_test");
    assert!(output.contains("done"));
    assert_message(&input[4], "assistant", "done");

    let report = store
        .preview_compact(
            "compact_preview",
            "strand_test",
            "message_first",
            "message_last",
        )
        .await
        .expect("preview");
    let preview = provider_preview(
        &store,
        "strand_test",
        &report,
        "summary",
        &serde_json::json!({"reason": "test"}),
    )
    .await
    .expect("preview assembly");
    assert_compact(&preview);
    store
        .create_compact(CompactDraft {
            tag: "compact_test",
            strand: "strand_test",
            first: "message_first",
            last: "message_last",
            summary: "summary",
            metadata: None,
            created: LATER,
        })
        .await
        .expect("compact");
    assert_compact(
        &provider_input(&store, "strand_test")
            .await
            .expect("compact assembly"),
    );
}

fn assert_message(item: &Item, expected_role: &str, expected_content: &str) {
    let Item::Message { role, content } = item else {
        panic!("expected message");
    };
    assert_eq!(role, expected_role);
    assert_eq!(content, expected_content);
}

fn assert_compact(items: &[Item]) {
    assert_eq!(items.len(), 1);
    let Item::Message { role, content } = &items[0] else {
        panic!("expected compact message");
    };
    assert_eq!(role, "system");
    assert!(content.contains("[compact projection]"));
    assert!(content.contains("<compact_summary>\nsummary\n</compact_summary>"));
}
