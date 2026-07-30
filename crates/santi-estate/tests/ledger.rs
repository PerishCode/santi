use santi_estate::{
    CallDraft, MessageDraft, ReplyDraft, Store, StrandDraft, ThinkingDraft, TurnDraft,
};
use santi_model::{message, thinking, turn};

const SUDO: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const NOW: &str = "2026-07-28T00:00:00.000Z";

#[tokio::test]
async fn ledger() {
    let temp = tempfile::tempdir().expect("temp");
    let store = Store::bootstrap(temp.path().join("estate.sqlite"), SUDO)
        .await
        .expect("open");
    let genesis = store.seed("soul_default", NOW).await.expect("seed");
    assert_eq!(genesis.id, "soul_default");
    assert_eq!(
        store.seed("soul_default", NOW).await.expect("reseed").id,
        genesis.id
    );

    let strand = store
        .create_strand(StrandDraft {
            tag: "ss_test",
            soul: &genesis.id,
            label: Some("primary"),
            parent: None,
            fork: None,
            created: NOW,
        })
        .await
        .expect("strand");
    assert_eq!(strand.next, 1);
    assert_eq!(strand.label.as_deref(), Some("primary"));

    let placed = store
        .place(MessageDraft {
            tag: "msg_test",
            strand: &strand.id,
            actor: message::Role::System,
            actor_id: "santi",
            kind: message::Kind::Text,
            content: &message::Content::text("hello"),
            state: message::State::Fixed,
            request: true,
            created: NOW,
        })
        .await
        .expect("place");
    assert_eq!(placed.relation.seq, 1);
    assert_eq!(placed.text, "hello");
    assert_eq!(
        store
            .strand(&strand.id)
            .await
            .expect("read")
            .expect("held")
            .next,
        2
    );
    assert_eq!(store.messages(&strand.id).await.expect("messages").len(), 1);

    let turn = store
        .create_turn(TurnDraft {
            tag: "turn_test",
            strand: &strand.id,
            trigger: turn::Trigger::StrandSend,
            source: Some("test"),
            from: 1,
            created: NOW,
        })
        .await
        .expect("turn");
    assert_eq!(turn.status, turn::Status::Running);
    assert_eq!(store.running().await.expect("running"), 1);

    let thinking = store
        .create_thinking(ThinkingDraft {
            tag: "thinking_test",
            turn: &turn.id,
            response: Some("response_test"),
            created: NOW,
        })
        .await
        .expect("thinking");
    store
        .update_thinking(&thinking.id, None, Some("summary"), NOW)
        .await
        .expect("summary");
    let thinking = store
        .complete_thinking(&thinking.id, thinking::Reason::Called, NOW)
        .await
        .expect("complete thinking")
        .expect("thinking");
    assert_eq!(thinking.state, thinking::State::Completed);

    let call = store
        .create_call(CallDraft {
            tag: "call_test",
            turn: &turn.id,
            tool: "shell",
            arguments: &serde_json::json!({"command": "true"}),
            created: NOW,
        })
        .await
        .expect("call");
    let reply = store
        .create_reply(ReplyDraft {
            tag: "result_test",
            call: &call.id,
            output: Some(&serde_json::json!({"ok": true})),
            error: None,
            created: NOW,
        })
        .await
        .expect("reply");
    assert_eq!(reply.output, Some(serde_json::json!({"ok": true})));
    assert!(
        store
            .create_reply(ReplyDraft {
                tag: "result_bad",
                call: &call.id,
                output: None,
                error: None,
                created: NOW,
            })
            .await
            .is_err()
    );

    let turn = store
        .complete_turn(&turn.id, 4, NOW)
        .await
        .expect("complete turn");
    assert_eq!(turn.status, turn::Status::Completed);
    assert_eq!(turn.to, Some(4));
    assert_eq!(store.running().await.expect("running"), 0);
    assert!(store.fail_turn(&turn.id, "late", NOW).await.is_err());

    let reopened = Store::open(temp.path().join("estate.sqlite"))
        .await
        .expect("reopen");
    assert_eq!(reopened.souls().await.expect("souls").len(), 1);
    assert_eq!(reopened.strands().await.expect("strands").len(), 1);
    assert_eq!(
        reopened.messages(&strand.id).await.expect("messages").len(),
        1
    );
}
