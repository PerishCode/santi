use super::support::*;

#[test]
fn appends_relations_in_order() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = SantiStore::open(temp.path().join("santi.sqlite")).expect("open store");
    let strand = store.create_strand().expect("create strand");
    let user = store
        .append_message(
            &strand.id,
            ActorType::System,
            store.system_actor_id(),
            MessageContent::text("hello ordering"),
            MessageState::Fixed,
            MessageIntake::Request,
        )
        .expect("append user")
        .strand_message;

    assert_eq!(user.relation.strand_seq, 1);
    let input = store.assembly_input(&strand.id).expect("assembly input");
    assert_eq!(input.len(), 1);
    assert_text(&input[0], "user", "hello ordering");
}

#[test]
fn maps_santi_system_input() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = SantiStore::open(temp.path().join("santi.sqlite")).expect("open store");
    let strand = store.create_strand().expect("create strand");
    let message = store
        .append_santi_system_message(
            &strand.id,
            MessageContent::text("<system_message>\nkind: note\n</system_message>"),
            MessageIntake::Request,
        )
        .expect("append santi system")
        .strand_message;

    assert_eq!(message.message.actor_type, ActorType::System);
    assert_eq!(message.message.message_kind, MessageKind::SantiSystem);
    let input = store.assembly_input(&strand.id).expect("assembly input");
    assert_eq!(input.len(), 1);
    assert_text(
        &input[0],
        "system",
        "<system_message>\nkind: note\n</system_message>",
    );
}

#[test]
fn thinking_becomes_reasoning() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = SantiStore::open(temp.path().join("santi.sqlite")).expect("open store");
    let strand = store.create_strand().expect("create strand");
    let user = store
        .append_message(
            &strand.id,
            ActorType::System,
            store.system_actor_id(),
            MessageContent::text("hello thinking"),
            MessageState::Fixed,
            MessageIntake::Request,
        )
        .expect("append user")
        .strand_message;
    let turn = store
        .start_turn(&strand.id, &user.message.id)
        .expect("start turn")
        .turn;
    let thinking = store
        .append_thinking_span(&turn.id, Some("resp_test".to_string()))
        .expect("append thinking");
    let thinking = store
        .update_thinking_span_summary(&thinking.id, "Looked at the prompt.".to_string())
        .expect("update thinking summary")
        .expect("thinking exists");
    let thinking = store
        .complete_thinking_span(&thinking.id, ThinkingCompletionReason::FirstTextDelta)
        .expect("complete thinking")
        .expect("thinking exists");

    let snapshot = store
        .runtime_snapshot(&strand.id)
        .expect("runtime snapshot")
        .expect("strand exists");
    assert_eq!(snapshot.thinking_spans.len(), 1);
    assert_eq!(snapshot.thinking_spans[0].id, thinking.id);
    assert_eq!(
        snapshot.thinking_spans[0].provider_response_id.as_deref(),
        Some("resp_test")
    );
    assert_eq!(
        snapshot.thinking_spans[0].summary.as_deref(),
        Some("Looked at the prompt.")
    );
    assert_eq!(
        snapshot.thinking_spans[0].completion_reason,
        Some(ThinkingCompletionReason::FirstTextDelta)
    );

    // Reasoning is now a first-class timeline item (adapters drop it per DC5,
    // but the projection includes it when there is real summary text).
    let input = store.assembly_input(&strand.id).expect("assembly input");
    assert_eq!(input.len(), 2);
    assert_text(&input[0], "user", "hello thinking");
    match &input[1] {
        ProviderItem::Reasoning { id, content } => {
            assert_eq!(id.as_deref(), Some("resp_test"));
            assert_eq!(content, "Looked at the prompt.");
        }
        other => panic!("expected reasoning item, got {other:?}"),
    }
}

#[test]
fn timeline_interleaves() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = SantiStore::open(temp.path().join("santi.sqlite")).expect("open store");
    let strand = store.create_strand().expect("create strand");

    // A turn with a shell tool roundtrip and per-round assistant text: the
    // replay timeline interleaves user, function call, result, assistant text.
    let user = store
        .append_message(
            &strand.id,
            ActorType::System,
            store.system_actor_id(),
            MessageContent::text("run a command"),
            MessageState::Fixed,
            MessageIntake::Request,
        )
        .expect("append user")
        .strand_message;
    let turn = store
        .start_turn(&strand.id, &user.message.id)
        .expect("start turn")
        .turn;
    store
        .append_tool_call(
            &turn.id,
            "call_1",
            "shell",
            &serde_json::json!({ "command": "echo hi" }),
            &ToolCallProvenance {
                provider_family: "openai".to_string(),
                item: Some(serde_json::json!({ "type": "function_call", "id": "fc_1" })),
                item_id: Some("fc_1".to_string()),
                response_id: Some("resp_1".to_string()),
            },
        )
        .expect("append tool call");
    store
        .append_tool_result(
            "call_1",
            Some(serde_json::json!({ "stdout": "hi\n" })),
            None,
        )
        .expect("append tool result");
    // The final assistant text is a soul-only timeline item (DC4b); the lumped
    // strand-visible reply is stored separately by the service.
    store
        .append_soul_assistant_text(&strand.id, "done")
        .expect("append soul assistant text");

    let input = store.assembly_input(&strand.id).expect("assembly input");
    assert_eq!(input.len(), 4);
    assert_text(&input[0], "user", "run a command");
    match &input[1] {
        ProviderItem::FunctionCall {
            call_id,
            name,
            arguments_raw,
            item,
            item_id,
        } => {
            assert_eq!(call_id, "call_1");
            assert_eq!(name, "shell");
            assert!(arguments_raw.contains("echo hi"));
            // The raw provider item + id round-trip for faithful Responses replay.
            assert_eq!(item_id.as_deref(), Some("fc_1"));
            assert_eq!(item.as_ref().expect("raw item")["id"], "fc_1");
        }
        other => panic!("expected function call, got {other:?}"),
    }
    match &input[2] {
        ProviderItem::FunctionCallOutput { call_id, output } => {
            assert_eq!(call_id, "call_1");
            assert!(output.contains("\"ok\":true"));
            assert!(output.contains("hi"));
        }
        other => panic!("expected function call output, got {other:?}"),
    }
    assert_text(&input[3], "assistant", "done");
}
