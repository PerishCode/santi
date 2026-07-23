use super::*;
use santi_core::{ingest, message, strand};

#[tokio::test]
async fn resume_holds_pending() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db = temp.path().join("santi.sqlite");
    let provider = Arc::new(FakeProvider {
        bytes: Some(1),
        ..FakeProvider::default()
    });
    let service = service_with_budget(&temp, provider.clone());
    let strand = service.weave().expect("create strand").strand;
    let store = Store::open(&db).expect("open store directly");
    let santi_core::ingest::Outcome::Accepted { .. } = store
        .receive(
            &strand.id,
            message::Kind::Text,
            message::Content::text("stranded pending that exceeds budget"),
            None,
        )
        .expect("offline enqueue")
    else {
        panic!("offline enqueue should be accepted before an incident exists");
    };
    drop(store);

    service.resume().expect("resume pending");
    let runtime = service
        .snapshot(&strand.id)
        .expect("runtime")
        .expect("strand");
    assert!(provider.requests.lock().unwrap().is_empty());
    assert!(runtime.messages.is_empty());
    assert!(runtime.turns.is_empty());
    assert_eq!(runtime.errors.len(), 1);
    assert_eq!(
        runtime.errors[0].first.context["reason"],
        "pending_drain_would_exceed_budget"
    );
    let conn = Connection::open(db).expect("open sqlite");
    let inbox: i64 = conn
        .query_row("SELECT COUNT(*) FROM strand_inbox", [], |row| row.get(0))
        .expect("inbox count");
    assert_eq!(
        inbox, 1,
        "pre-incident accepted input must remain recoverable"
    );
}

#[tokio::test]
async fn rejected_payload_is_ephemeral() {
    let temp = tempfile::tempdir().expect("temp dir");
    let provider = Arc::new(FakeProvider {
        bytes: Some(1),
        ..FakeProvider::default()
    });
    let service = service_with_budget(&temp, provider);
    let soul = service.souls().expect("souls")[0].id.clone();
    let source = ingest::Source::new("test").with_metadata(json!({
        "raw": "SOURCE_SECRET_MARKER".repeat(600),
    }));
    let outcome = service
        .sourced(
            &soul,
            "test:no-payload-audit",
            "MESSAGE_SECRET_MARKER".repeat(500),
            Some(source),
        )
        .expect("ingest");
    let santi_core::ingest::Outcome::Rejected { .. } = outcome else {
        panic!("expected over-budget rejection");
    };

    let strand = service
        .strands()
        .expect("strands")
        .into_iter()
        .find(|strand| strand.label.as_deref() == Some("test:no-payload-audit"))
        .expect("labeled strand");
    let incident = service
        .stranded(&strand.id, 10)
        .expect("errors")
        .expect("strand")
        .pop()
        .expect("incident");
    let serialized = serde_json::to_string(&incident).expect("serialize incident");
    assert!(!serialized.contains("SOURCE_SECRET_MARKER"));
    assert!(!serialized.contains("MESSAGE_SECRET_MARKER"));
}

#[tokio::test]
async fn compact_resolves_incident() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db = temp.path().join("santi.sqlite");
    let provider = Arc::new(LargeToolCallProvider {
        requests: Arc::new(Mutex::new(Vec::new())),
        bytes: 100_000,
    });
    let service = service_with_budget(&temp, provider.clone());
    let strand = service.weave().expect("create strand").strand;
    let response = service
        .send(
            &strand.id,
            strand::Post {
                content: vec![message::Part::Text {
                    text: "please run the large tool".to_string(),
                }],
            },
        )
        .await
        .expect("send strand");

    let runtime = Probe::new(&service)
        .failed_turn(&strand.id, &accepted_turn(&response).id)
        .await;
    assert_eq!(provider.requests.lock().unwrap().len(), 1);
    assert_eq!(runtime.errors.len(), 1);
    assert_eq!(runtime.errors[0].status, santi_core::Status::Active);
    assert_eq!(
        runtime.errors[0].first.context["reason"],
        "provider_request_exceeds_budget"
    );
    assert!(
        runtime
            .messages
            .iter()
            .all(|message| message.message.kind != message::Kind::SantiSystem),
        "budget preflight must not project a model-visible notice"
    );

    let first = runtime
        .messages
        .iter()
        .find(|message| message.text == "please run the large tool")
        .expect("user message")
        .message
        .id
        .clone();
    let conn = Connection::open(&db).expect("open sqlite");
    conn.execute(
        r#"
        INSERT INTO strand_inbox (
          id, strand_id, message_kind, content, source_type, source_ref,
          source_metadata, created_at
        ) VALUES (?1, ?2, 'text', ?3, 'test', 'accepted-before-incident', NULL, ?4)
        "#,
        rusqlite::params![
            "inbox_before_incident",
            strand.id,
            serde_json::to_string(&message::Content::text("deliver after recovery"))
                .expect("serialize inbox content"),
            "2026-07-10T00:00:00Z",
        ],
    )
    .expect("seed pre-incident accepted input");
    drop(conn);
    let store = Store::open(&db).expect("open store directly");
    let boundary = store
        .pen(Draft {
            strand: &strand.id,
            actor: message::Role::Soul,
            id: store.genesis(),
            content: message::Content::text("manual compact boundary"),
            state: message::State::Fixed,
            intake: message::Intake::Record,
        })
        .expect("append manual boundary")
        .message;

    let compact = service
        .exec(
            &strand.id,
            santi_core::compact::Exec {
                first: Some(first),
                last: Some(boundary.message.id),
                from: None,
                to: None,
                summary: "Large tool exchange collapsed after context-budget incident.".to_string(),
                capsule: None,
                dry: false,
            },
        )
        .expect("compact should resolve incident when estimate is under budget");
    assert!(compact.active_incident_resolved);

    let after = Probe::new(&service).any_completed(&strand.id).await;
    assert_eq!(after.errors.len(), 1);
    assert_eq!(after.errors[0].status, santi_core::Status::Resolved);
    assert_eq!(after.errors[0].revision, 2);
    assert_eq!(
        after.errors[0].resolution.as_ref().unwrap().by.as_deref(),
        Some("compact_exec")
    );
    assert!(
        after
            .messages
            .iter()
            .any(|message| message.text == "deliver after recovery"),
        "explicit resolution should resume accepted pending input"
    );
    let conn = Connection::open(db).expect("open sqlite");
    let transitions: i64 = conn
        .query_row("SELECT COUNT(*) FROM error_transitions", [], |row| {
            row.get(0)
        })
        .expect("transition count");
    assert_eq!(
        transitions, 2,
        "open and resolve are the only lifecycle events"
    );
}

#[tokio::test]
async fn budget_raise_clears_hold_on_ingest() {
    let temp = tempfile::tempdir().expect("temp dir");
    let held = service_with_budget(
        &temp,
        Arc::new(FakeProvider {
            bytes: Some(1),
            ..FakeProvider::default()
        }),
    );
    let rejected = held
        .evented(
            santi_core::GENESIS,
            "test:operator",
            "over budget".to_string(),
        )
        .expect("external ingest");
    let santi_core::ingest::Outcome::Rejected { error } = rejected else {
        panic!("first send should open the hold");
    };
    assert_eq!(error.code, "context.budget.exceeded");
    let repeat = held
        .evented(
            santi_core::GENESIS,
            "test:operator",
            "still held".to_string(),
        )
        .expect("external ingest repeat");
    let santi_core::ingest::Outcome::Rejected {
        error: repeat_error,
    } = repeat
    else {
        panic!("held strand must keep rejecting before the raise");
    };
    assert_eq!(repeat_error.code, "context.budget.exceeded");
    drop(held);

    let raised = service_with_budget(
        &temp,
        Arc::new(FakeProvider {
            bytes: Some(500_000),
            ..FakeProvider::default()
        }),
    );
    let outcome = raised
        .evented(
            santi_core::GENESIS,
            "test:operator",
            "after the raise".to_string(),
        )
        .expect("external ingest after raise");
    let santi_core::ingest::Outcome::Accepted { receipt } = outcome else {
        panic!("under-budget hold must auto-clear on ingest remeasure");
    };
    let runtime = raised
        .snapshot(&receipt.strand)
        .expect("runtime")
        .expect("strand");
    let incident = runtime
        .errors
        .iter()
        .find(|incident| incident.code == "context.budget.exceeded")
        .expect("context incident recorded");
    assert_eq!(incident.status, santi_core::Status::Resolved);
}
