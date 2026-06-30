use rusqlite::Connection;
use santi_core::{
    ActorType, MessageContent, MessageKind, MessageState, ProviderMessage, SantiStore,
    ThinkingCompletionReason,
};

fn assert_text(message: &ProviderMessage, role: &str, content: &str) {
    match message {
        ProviderMessage::Text {
            role: actual_role,
            content: actual_content,
        } => {
            assert_eq!(actual_role, role);
            assert_eq!(actual_content, content);
        }
        other => panic!("expected text message, got {other:?}"),
    }
}

#[test]
fn schema_matches_runtime() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db = temp.path().join("santi.sqlite");
    let store = SantiStore::open(&db).expect("open store");
    drop(store);

    let conn = Connection::open(db).expect("open sqlite");
    for table in [
        "accounts",
        "souls",
        "soul_profiles",
        "sessions",
        "session_profiles",
        "messages",
        "r_session_messages",
        "message_events",
        "session_effects",
        "soul_sessions",
        "turns",
        "tool_calls",
        "tool_results",
        "thinking_spans",
        "compacts",
        "r_soul_session_messages",
    ] {
        let exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table],
                |row| row.get(0),
            )
            .expect("table lookup");
        assert_eq!(exists, 1, "missing table {table}");
    }
}

#[test]
fn appends_relations_in_order() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = SantiStore::open(temp.path().join("santi.sqlite")).expect("open store");
    let session = store.create_session().expect("create session");
    let user = store
        .append_message(
            &session.session.id,
            ActorType::Account,
            store.default_account_id(),
            MessageContent::text("hello ordering"),
            MessageState::Fixed,
        )
        .expect("append user")
        .session_message;
    let soul_session = store
        .acquire_soul_session(&session.session.id)
        .expect("acquire soul session")
        .soul_session;
    let entry = store
        .append_message_ref(&soul_session.id, &user.message.id)
        .expect("append message ref");

    assert_eq!(user.relation.session_seq, 1);
    assert_eq!(entry.soul_session_seq, 1);
    let input = store
        .assembly_input(&soul_session.id, i64::MAX)
        .expect("assembly input");
    assert_eq!(input.len(), 1);
    assert_text(&input[0], "user", "hello ordering");
}

#[test]
fn maps_santi_system_input() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = SantiStore::open(temp.path().join("santi.sqlite")).expect("open store");
    let session = store.create_session().expect("create session");
    let soul_session = store
        .acquire_soul_session(&session.session.id)
        .expect("acquire soul session")
        .soul_session;
    let message = store
        .append_santi_system_message(
            &session.session.id,
            MessageContent::text("<santi-system>\nkind: note\n</santi-system>"),
        )
        .expect("append santi system")
        .session_message;
    store
        .append_message_ref(&soul_session.id, &message.message.id)
        .expect("append message ref");

    assert_eq!(message.message.actor_type, ActorType::System);
    assert_eq!(message.message.message_kind, MessageKind::SantiSystem);
    let input = store
        .assembly_input(&soul_session.id, i64::MAX)
        .expect("assembly input");
    assert_eq!(input.len(), 1);
    assert_text(
        &input[0],
        "user",
        "<santi-system>\nkind: note\n</santi-system>",
    );
}

#[test]
fn thinking_spans_skip_input() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = SantiStore::open(temp.path().join("santi.sqlite")).expect("open store");
    let session = store.create_session().expect("create session");
    let user = store
        .append_message(
            &session.session.id,
            ActorType::Account,
            store.default_account_id(),
            MessageContent::text("hello thinking"),
            MessageState::Fixed,
        )
        .expect("append user")
        .session_message;
    let soul_session = store
        .acquire_soul_session(&session.session.id)
        .expect("acquire soul session")
        .soul_session;
    store
        .append_message_ref(&soul_session.id, &user.message.id)
        .expect("append message ref");
    let turn = store
        .start_turn(
            &soul_session.id,
            &user.message.id,
            user.relation.session_seq,
        )
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
        .runtime_snapshot(&session.session.id)
        .expect("runtime snapshot")
        .expect("session exists");
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

    let input = store
        .assembly_input(&soul_session.id, i64::MAX)
        .expect("assembly input");
    assert_eq!(input.len(), 1);
    assert_text(&input[0], "user", "hello thinking");
}

#[test]
fn titles_from_first_message() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = SantiStore::open(temp.path().join("santi.sqlite")).expect("open store");
    let session = store.create_session().expect("create session");
    assert_eq!(session.profile.title, None);
    let title = "first visible session title with enough detail to remain durable";

    store
        .append_message(
            &session.session.id,
            ActorType::Account,
            store.default_account_id(),
            MessageContent::text(format!("  {title}  ")),
            MessageState::Fixed,
        )
        .expect("append first message");
    store
        .append_message(
            &session.session.id,
            ActorType::Account,
            store.default_account_id(),
            MessageContent::text("should not replace title"),
            MessageState::Fixed,
        )
        .expect("append second message");

    let session = store
        .session(&session.session.id)
        .expect("load session")
        .expect("session exists");
    let profile = store
        .runtime_snapshot(&session.id)
        .expect("runtime snapshot")
        .expect("session exists")
        .profile;
    assert_eq!(profile.title.as_deref(), Some(title));
}

#[test]
fn trims_session_title() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = SantiStore::open(temp.path().join("santi.sqlite")).expect("open store");
    let session = store.create_session().expect("create session");

    let session = store
        .update_session_title(&session.session.id, Some("  renamed title  ".to_string()))
        .expect("update title")
        .expect("session exists");
    assert_eq!(session.profile.title.as_deref(), Some("renamed title"));

    let session = store
        .update_session_title(&session.session.id, Some("   ".to_string()))
        .expect("clear title")
        .expect("session exists");
    assert_eq!(session.profile.title, None);
}

#[test]
fn replays_completed_tool_history_but_not_in_flight() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = SantiStore::open(temp.path().join("santi.sqlite")).expect("open store");
    let session = store.create_session().expect("create session");

    // Turn 1: user message, a completed shell tool roundtrip, and the answer.
    let user = store
        .append_message(
            &session.session.id,
            ActorType::Account,
            store.default_account_id(),
            MessageContent::text("run a command"),
            MessageState::Fixed,
        )
        .expect("append user")
        .session_message;
    let soul_session = store
        .acquire_soul_session(&session.session.id)
        .expect("acquire soul session")
        .soul_session;
    store
        .append_message_ref(&soul_session.id, &user.message.id)
        .expect("append user ref");
    let turn1 = store
        .start_turn(
            &soul_session.id,
            &user.message.id,
            user.relation.session_seq,
        )
        .expect("start turn1")
        .turn;
    store
        .append_tool_call(
            &turn1.id,
            "call_1",
            "shell",
            &serde_json::json!({ "command": "echo hi" }),
        )
        .expect("append tool call");
    store
        .append_tool_result(
            "call_1",
            Some(serde_json::json!({ "stdout": "hi\n" })),
            None,
        )
        .expect("append tool result");
    let answer = store
        .append_message(
            &session.session.id,
            ActorType::Soul,
            store.default_soul_id(),
            MessageContent::text("done"),
            MessageState::Fixed,
        )
        .expect("append answer")
        .session_message;
    store
        .append_message_ref(&soul_session.id, &answer.message.id)
        .expect("append answer ref");

    // Turn 2 opens: the turn-1 tool roundtrip is now completed history.
    let follow_up = store
        .append_message(
            &session.session.id,
            ActorType::Account,
            store.default_account_id(),
            MessageContent::text("and again"),
            MessageState::Fixed,
        )
        .expect("append follow up")
        .session_message;
    store
        .append_message_ref(&soul_session.id, &follow_up.message.id)
        .expect("append follow up ref");
    let turn2 = store
        .start_turn(
            &soul_session.id,
            &follow_up.message.id,
            follow_up.relation.session_seq,
        )
        .expect("start turn2")
        .turn;

    // With turn-2's base as the boundary, turn-1's tools are replayed inline:
    // user, assistant tool_calls, tool result, assistant answer, follow-up user.
    let base2 = store
        .turn_base_soul_session_seq(&turn2.id)
        .expect("turn2 base seq");
    let input = store
        .assembly_input(&soul_session.id, base2)
        .expect("assembly input replay");
    assert_eq!(input.len(), 5);
    assert_text(&input[0], "user", "run a command");
    match &input[1] {
        ProviderMessage::ToolCalls { calls } => {
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].call_id, "call_1");
            assert_eq!(calls[0].name, "shell");
            assert!(calls[0].arguments_raw.contains("echo hi"));
        }
        other => panic!("expected tool calls, got {other:?}"),
    }
    match &input[2] {
        ProviderMessage::ToolResult { call_id, content } => {
            assert_eq!(call_id, "call_1");
            assert!(content.contains("\"ok\":true"));
            assert!(content.contains("hi"));
        }
        other => panic!("expected tool result, got {other:?}"),
    }
    assert_text(&input[3], "assistant", "done");
    assert_text(&input[4], "user", "and again");

    // With turn-1's base as the boundary, that turn's own tools are in-flight
    // and excluded (the service drives them via function_call_outputs instead).
    let base1 = store
        .turn_base_soul_session_seq(&turn1.id)
        .expect("turn1 base seq");
    let in_flight = store
        .assembly_input(&soul_session.id, base1)
        .expect("assembly input in flight");
    assert!(
        in_flight.iter().all(|message| !matches!(
            message,
            ProviderMessage::ToolCalls { .. } | ProviderMessage::ToolResult { .. }
        )),
        "in-flight tool entries must be excluded"
    );
}
