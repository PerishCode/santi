use santi_estate::{
    CallDraft, CompactDraft, MessageDraft, ReplyDraft, Store, StrandDraft, TurnDraft,
};
use santi_model::{message, turn};

const FIRST: &str = "2026-07-28T00:00:00.000Z";
const LATER: &str = "2026-07-28T00:01:00.000Z";

#[tokio::test]
async fn compacts() {
    let temp = tempfile::tempdir().expect("temp");
    let path = temp.path().join("estate.sqlite");
    let store = super::support::bootstrap(&path).await;
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
    let first = message::Content::text("first boundary");
    store
        .place(MessageDraft {
            tag: "message_first",
            strand: "strand_test",
            actor: message::Role::System,
            actor_id: "santi",
            kind: message::Kind::Text,
            content: &first,
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
        .create_call(CallDraft {
            tag: "call_test",
            turn: "turn_test",
            tool: "shell",
            arguments: &serde_json::json!({"command": "true"}),
            created: FIRST,
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
        .expect("result");
    let last = message::Content::text("last boundary");
    store
        .place(MessageDraft {
            tag: "message_last",
            strand: "strand_test",
            actor: message::Role::Soul,
            actor_id: "soul_test",
            kind: message::Kind::Text,
            content: &last,
            state: message::State::Fixed,
            request: false,
            created: LATER,
        })
        .await
        .expect("last");

    let preview = store
        .preview_compact(
            "compact_preview",
            "strand_test",
            "message_first",
            "message_last",
        )
        .await
        .expect("preview");
    assert!(preview.dry);
    assert_eq!((preview.from, preview.to, preview.collapsed), (1, 4, 4));
    assert_eq!(preview.first, "message_first");
    assert_eq!(preview.last, "message_last");

    store
        .create_compact(CompactDraft {
            tag: "compact_inner",
            strand: "strand_test",
            first: "message_first",
            last: "message_first",
            summary: "inner",
            metadata: None,
            created: FIRST,
        })
        .await
        .expect("inner");
    let metadata = serde_json::json!({"final": true});
    let outer = store
        .create_compact(CompactDraft {
            tag: "compact_outer",
            strand: "strand_test",
            first: "message_first",
            last: "message_last",
            summary: "outer",
            metadata: Some(&metadata),
            created: LATER,
        })
        .await
        .expect("outer");
    assert_eq!(outer.absorbed, vec!["compact_inner"]);
    assert_eq!(
        store.compacts("strand_test").await.expect("compacts").len(),
        1
    );
    let held = store
        .compact("compact_outer")
        .await
        .expect("compact")
        .expect("held");
    assert_eq!(held.metadata, Some(metadata));
    assert!(
        store
            .create_compact(CompactDraft {
                tag: "compact_partial",
                strand: "strand_test",
                first: "message_last",
                last: "message_last",
                summary: "partial",
                metadata: None,
                created: LATER,
            })
            .await
            .is_err()
    );

    let page = store
        .compact_page("compact_outer", None, 0, 10)
        .await
        .expect("page")
        .expect("held");
    assert_eq!(page.total, 4);
    assert_eq!(page.entries[0].text, "first boundary");
    assert!(page.entries[1].text.contains("[tool_call shell]"));
    assert!(page.entries[2].text.contains("[tool_result]"));
    assert_eq!(page.entries[3].text, "last boundary");
    assert_eq!(
        store
            .compact_page("compact_outer", Some("done"), 0, 10)
            .await
            .expect("search")
            .expect("held")
            .total,
        1
    );
    assert_eq!(
        store
            .seated("strand_test", 4)
            .await
            .expect("seat")
            .as_deref(),
        Some("message_last")
    );

    drop(store);
    let store = Store::open(path).await.expect("open again");
    assert!(
        store
            .compact("compact_outer")
            .await
            .expect("compact again")
            .is_some()
    );
}
