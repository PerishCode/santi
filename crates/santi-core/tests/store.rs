use rusqlite::Connection;
use santi_core::{
    ActorType, MessageContent, MessageKind, MessageState, SantiStore, ThinkingCompletionReason,
};

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
        .assembly_input(&soul_session.id)
        .expect("assembly input");
    assert_eq!(input.len(), 1);
    assert_eq!(input[0].role, "user");
    assert_eq!(input[0].content, "hello ordering");
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
        .assembly_input(&soul_session.id)
        .expect("assembly input");
    assert_eq!(input.len(), 1);
    assert_eq!(input[0].role, "user");
    assert_eq!(
        input[0].content,
        "<santi-system>\nkind: note\n</santi-system>"
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
        .assembly_input(&soul_session.id)
        .expect("assembly input");
    assert_eq!(input.len(), 1);
    assert_eq!(input[0].content, "hello thinking");
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
