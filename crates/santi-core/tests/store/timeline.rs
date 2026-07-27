use super::support::*;
use santi_core::{message, thinking, tool};

#[test]
fn appends() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = Store::open(temp.path().join("santi.sqlite")).expect("open store");
    let strand = store.weave().expect("create strand");
    let user = store
        .pen(Draft {
            strand: &strand.id,
            actor: message::Role::System,
            id: store.system(),
            content: message::Content::text("hello ordering"),
            state: message::State::Fixed,
            intake: message::Intake::Request,
        })
        .expect("append user")
        .message;

    assert_eq!(user.relation.seq, 1);
    let input = store.assembly(&strand.id).expect("assembly input");
    assert_eq!(input.len(), 1);
    assert_text(&input[0], "user", "hello ordering");
}

#[test]
fn maps() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = Store::open(temp.path().join("santi.sqlite")).expect("open store");
    let strand = store.weave().expect("create strand");
    let message = store
        .inscribe(
            &strand.id,
            message::Content::text("<system_message>\nkind: note\n</system_message>"),
            message::Intake::Request,
        )
        .expect("append santi system")
        .message;

    assert_eq!(message.message.role, message::Role::System);
    assert_eq!(message.message.kind, message::Kind::SantiSystem);
    let input = store.assembly(&strand.id).expect("assembly input");
    assert_eq!(input.len(), 1);
    assert_text(
        &input[0],
        "system",
        "<system_message>\nkind: note\n</system_message>",
    );
}

#[test]
fn reasons() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = Store::open(temp.path().join("santi.sqlite")).expect("open store");
    let strand = store.weave().expect("create strand");
    let user = store
        .pen(Draft {
            strand: &strand.id,
            actor: message::Role::System,
            id: store.system(),
            content: message::Content::text("hello thinking"),
            state: message::State::Fixed,
            intake: message::Intake::Request,
        })
        .expect("append user")
        .message;
    let turn = store
        .start(&strand.id, &user.message.id)
        .expect("start turn")
        .turn;
    let thinking = store
        .muse(&turn.id, Some("resp_test".to_string()))
        .expect("append thinking");
    let thinking = store
        .summarize(&thinking.id, "Looked at the prompt.".to_string())
        .expect("update thinking summary")
        .expect("thinking exists");
    let thinking = store
        .conclude(&thinking.id, thinking::Reason::Spoke)
        .expect("complete thinking")
        .expect("thinking exists");

    let snapshot = store
        .snapshot(&strand.id)
        .expect("runtime snapshot")
        .expect("strand exists");
    assert_eq!(snapshot.thinking.len(), 1);
    assert_eq!(snapshot.thinking[0].id, thinking.id);
    assert_eq!(snapshot.thinking[0].response.as_deref(), Some("resp_test"));
    assert_eq!(
        snapshot.thinking[0].summary.as_deref(),
        Some("Looked at the prompt.")
    );
    assert_eq!(
        snapshot.thinking[0].completion_reason,
        Some(thinking::Reason::Spoke)
    );

    let input = store.assembly(&strand.id).expect("assembly input");
    assert_eq!(input.len(), 2);
    assert_text(&input[0], "user", "hello thinking");
    match &input[1] {
        Item::Reasoning { id, content } => {
            assert_eq!(id.as_deref(), Some("resp_test"));
            assert_eq!(content, "Looked at the prompt.");
        }
        other => panic!("expected reasoning item, got {other:?}"),
    }
}

#[test]
fn interleaves() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = Store::open(temp.path().join("santi.sqlite")).expect("open store");
    let strand = store.weave().expect("create strand");

    let user = store
        .pen(Draft {
            strand: &strand.id,
            actor: message::Role::System,
            id: store.system(),
            content: message::Content::text("run a command"),
            state: message::State::Fixed,
            intake: message::Intake::Request,
        })
        .expect("append user")
        .message;
    let turn = store
        .start(&strand.id, &user.message.id)
        .expect("start turn")
        .turn;
    store
        .call(Invocation {
            turn: &turn.id,
            call: "call_1",
            name: "shell",
            arguments: &serde_json::json!({ "command": "echo hi" }),
            provenance: &tool::Provenance {
                family: "openai".to_string(),
                item: Some(serde_json::json!({ "type": "function_call", "id": "fc_1" })),
                mark: Some("fc_1".to_string()),
                response: Some("resp_1".to_string()),
            },
        })
        .expect("append tool call");
    store
        .reply(
            "call_1",
            Some(serde_json::json!({ "stdout": "hi\n" })),
            None,
        )
        .expect("append tool result");
    store
        .voice(&strand.id, "done")
        .expect("append soul assistant text");

    let input = store.assembly(&strand.id).expect("assembly input");
    assert_eq!(input.len(), 4);
    assert_text(&input[0], "user", "run a command");
    match &input[1] {
        Item::Call {
            call,
            name,
            raw,
            item,
            mark,
        } => {
            assert_eq!(call, "call_1");
            assert_eq!(name, "shell");
            assert!(raw.contains("echo hi"));
            assert_eq!(mark.as_deref(), Some("fc_1"));
            assert_eq!(item.as_ref().expect("raw item")["id"], "fc_1");
        }
        other => panic!("expected function call, got {other:?}"),
    }
    match &input[2] {
        Item::Output { call, output } => {
            assert_eq!(call, "call_1");
            assert!(output.contains("\"ok\":true"));
            assert!(output.contains("hi"));
        }
        other => panic!("expected function call output, got {other:?}"),
    }
    assert_text(&input[3], "assistant", "done");
}
