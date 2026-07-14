use super::support::*;
use rusqlite::Connection;
use santi_core::service::{self, Service};

#[path = "budget/memory_pressure.rs"]
mod memory_pressure;
#[path = "budget/recovery.rs"]
mod recovery;

fn service_with_budget(temp: &tempfile::TempDir, provider: Arc<dyn ProviderClient>) -> Service {
    Service::open(
        service::Config {
            database_path: temp.path().join("santi.sqlite").display().to_string(),
            runtime_root: temp.path().join("runtime").display().to_string(),
            execution_root: temp.path().join("execution").display().to_string(),
            bind_addr: Some("127.0.0.1:0".to_string()),
        },
        provider,
    )
    .expect("open service")
}

#[tokio::test]
async fn admission_opens_incident() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db = temp.path().join("santi.sqlite");
    let provider = Arc::new(FakeProvider {
        input_budget_bytes: Some(1),
        ..FakeProvider::default()
    });
    let service = service_with_budget(&temp, provider.clone());
    let mut events = service.subscribe_stream();
    let strand = service.create_strand().expect("create strand").strand;

    let error = service
        .send_strand(
            &strand.id,
            SendStrandRequest {
                content: vec![MessagePart::Text {
                    text: "this should not enter the strand".to_string(),
                }],
            },
        )
        .await
        .expect_err("send should be rejected");

    assert_eq!(error.code, "context.budget.exceeded");
    let incident_id = error.incident_id.expect("incident id");
    assert!(error.message.contains("strand context is over budget"));
    assert!(provider.requests.lock().unwrap().is_empty());
    let runtime = service
        .runtime_snapshot(&strand.id)
        .expect("runtime")
        .expect("strand");
    assert!(runtime.messages.is_empty(), "rejected send entered spine");
    assert!(runtime.turns.is_empty(), "rejected send started a turn");
    assert_eq!(runtime.errors.len(), 1);
    let incident = &runtime.errors[0];
    assert_eq!(incident.id, incident_id);
    assert_eq!(incident.status, santi_core::IncidentStatus::Active);
    assert_eq!(incident.occurrence_count, 1);
    assert_eq!(incident.revision, 1);
    assert_eq!(incident.source.operation, "ingest_admission");
    assert_eq!(incident.context["reason"], "candidate_input_exceeds_budget");

    let transition = std::iter::from_fn(|| events.try_recv().ok())
        .find_map(|event| match event.payload {
            santi_core::SantiStreamPayload::ErrorTransition { transition } => Some(transition),
            _ => None,
        })
        .expect("error transition event");
    assert_eq!(transition.incident_id, incident_id);
    assert_eq!(transition.revision, 1);
    assert_eq!(transition.kind, santi_core::ErrorTransitionKind::Opened);

    let conn = Connection::open(db).expect("open sqlite");
    let delivered: (i64, i64) = conn
        .query_row(
            "SELECT COUNT(*), COUNT(delivered_at) FROM error_transitions",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("transition counts");
    assert_eq!(delivered, (1, 1));
}

#[tokio::test]
async fn remeasures_hot_memory() {
    let temp = tempfile::tempdir().expect("temp dir");
    let runtime_root = temp.path().join("runtime");
    let memory_path = runtime_root.join("souls/soul_default/memory/MEMORY.md");
    fs::create_dir_all(memory_path.parent().expect("memory parent")).expect("create memory parent");
    fs::write(&memory_path, "m".repeat(9_000)).expect("write initial memory");
    let provider = Arc::new(FakeProvider {
        input_budget_bytes: Some(20_000),
        ..FakeProvider::default()
    });
    let service = Service::open(
        service::Config {
            database_path: temp.path().join("santi.sqlite").display().to_string(),
            runtime_root: runtime_root.display().to_string(),
            execution_root: temp.path().join("execution").display().to_string(),
            bind_addr: Some("127.0.0.1:0".to_string()),
        },
        provider.clone(),
    )
    .expect("open service");
    let strand = service.create_strand().expect("create strand").strand;

    let error = service
        .send_strand(
            &strand.id,
            SendStrandRequest {
                content: vec![MessagePart::Text {
                    text: "x".repeat(15_000),
                }],
            },
        )
        .await
        .expect_err("first candidate exceeds context budget");
    assert_eq!(error.code, "context.budget.exceeded");
    assert!(provider.requests.lock().unwrap().is_empty());

    fs::write(&memory_path, "# Small memory\n").expect("shrink memory directly");
    let accepted = service
        .send_strand(
            &strand.id,
            SendStrandRequest {
                content: vec![MessagePart::Text {
                    text: "new request after material relief".to_string(),
                }],
            },
        )
        .await
        .expect("hot memory change should clear stale incident");
    let turn = accepted.turn.expect("relieved request starts turn");
    Probe::new(&service)
        .completed_turn(&strand.id, &turn.id)
        .await;

    assert_eq!(provider.requests.lock().unwrap().len(), 1);
    assert!(
        service
            .strand_budget(&strand.id)
            .expect("strand budget")
            .expect("strand")
            .active_incident
            .is_none()
    );
    let incidents = service
        .strand_errors(&strand.id, 10)
        .expect("strand errors")
        .expect("strand");
    assert_eq!(incidents.len(), 1);
    assert_eq!(incidents[0].status, santi_core::IncidentStatus::Resolved);
}

#[tokio::test]
async fn repeats_are_idempotent() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db = temp.path().join("santi.sqlite");
    let provider = Arc::new(FakeProvider {
        input_budget_bytes: Some(1),
        ..FakeProvider::default()
    });
    let service = service_with_budget(&temp, provider);
    let strand = service.create_strand().expect("create strand").strand;

    let mut incident_ids = Vec::new();
    for text in ["first rejected", "second rejected", "third rejected"] {
        let error = service
            .send_strand(
                &strand.id,
                SendStrandRequest {
                    content: vec![MessagePart::Text {
                        text: text.to_string(),
                    }],
                },
            )
            .await
            .expect_err("send should be rejected");
        incident_ids.push(error.incident_id.expect("incident id"));
    }
    assert!(incident_ids.windows(2).all(|ids| ids[0] == ids[1]));

    let runtime = service
        .runtime_snapshot(&strand.id)
        .expect("runtime")
        .expect("strand");
    assert!(runtime.messages.is_empty());
    assert!(runtime.turns.is_empty());
    assert_eq!(runtime.errors.len(), 1);
    assert_eq!(runtime.errors[0].occurrence_count, 3);
    assert_eq!(runtime.errors[0].revision, 1);
    assert_eq!(
        runtime.errors[0].latest_source.operation,
        "ingest_active_guard"
    );

    let conn = Connection::open(db).expect("open sqlite");
    let transitions: i64 = conn
        .query_row("SELECT COUNT(*) FROM error_transitions", [], |row| {
            row.get(0)
        })
        .expect("transition count");
    let inbox: i64 = conn
        .query_row("SELECT COUNT(*) FROM strand_inbox", [], |row| row.get(0))
        .expect("inbox count");
    assert_eq!(transitions, 1, "repeats must not emit lifecycle rows");
    assert_eq!(inbox, 0, "rejected writes must not append inbox rows");
    let delivered: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM error_transitions WHERE delivered_at IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .expect("delivered count");
    assert_eq!(delivered, 0, "outbox must wait for a live bus consumer");

    let mut events = service.subscribe_stream();
    let event = events.try_recv().expect("pending transition");
    assert!(matches!(
        event.payload,
        santi_core::SantiStreamPayload::ErrorTransition { .. }
    ));
    let delivered: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM error_transitions WHERE delivered_at IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .expect("delivered count");
    assert_eq!(delivered, 1);
}

#[tokio::test]
async fn store_cannot_bypass() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db = temp.path().join("santi.sqlite");
    let provider = Arc::new(FakeProvider {
        input_budget_bytes: Some(1),
        ..FakeProvider::default()
    });
    let service = service_with_budget(&temp, provider);
    let strand = service.create_strand().expect("create strand").strand;
    let first = service
        .send_strand(
            &strand.id,
            SendStrandRequest {
                content: vec![MessagePart::Text {
                    text: "first rejected".to_string(),
                }],
            },
        )
        .await
        .expect_err("send should be rejected");

    let store = SantiStore::open(&db).expect("open store directly");
    let outcome = store
        .enqueue_inbox(
            &strand.id,
            MessageKind::Text,
            MessageContent::text("direct bypass attempt"),
        )
        .expect("direct enqueue");
    let santi_core::IngestOutcome::Rejected { error } = outcome else {
        panic!("direct enqueue should be rejected by the active incident");
    };
    assert_eq!(error.incident_id, first.incident_id);
    let runtime = service
        .runtime_snapshot(&strand.id)
        .expect("runtime")
        .expect("strand");
    assert_eq!(runtime.errors[0].occurrence_count, 2);
    assert!(runtime.messages.is_empty());
}

#[tokio::test]
async fn resume_holds_pending() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db = temp.path().join("santi.sqlite");
    let provider = Arc::new(FakeProvider {
        input_budget_bytes: Some(1),
        ..FakeProvider::default()
    });
    let service = service_with_budget(&temp, provider.clone());
    let strand = service.create_strand().expect("create strand").strand;
    let store = SantiStore::open(&db).expect("open store directly");
    let santi_core::IngestOutcome::Accepted { .. } = store
        .enqueue_inbox(
            &strand.id,
            MessageKind::Text,
            MessageContent::text("stranded pending that exceeds budget"),
        )
        .expect("offline enqueue")
    else {
        panic!("offline enqueue should be accepted before an incident exists");
    };
    drop(store);

    service.resume_pending().expect("resume pending");
    let runtime = service
        .runtime_snapshot(&strand.id)
        .expect("runtime")
        .expect("strand");
    assert!(provider.requests.lock().unwrap().is_empty());
    assert!(runtime.messages.is_empty());
    assert!(runtime.turns.is_empty());
    assert_eq!(runtime.errors.len(), 1);
    assert_eq!(
        runtime.errors[0].context["reason"],
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
        input_budget_bytes: Some(1),
        ..FakeProvider::default()
    });
    let service = service_with_budget(&temp, provider);
    let soul_id = service.list_souls().expect("souls")[0].id.clone();
    let source = InboxSource::new("test").with_metadata(json!({
        "raw": "SOURCE_SECRET_MARKER".repeat(600),
    }));
    let outcome = service
        .ingest_external_source(
            &soul_id,
            "test:no-payload-audit",
            "MESSAGE_SECRET_MARKER".repeat(500),
            Some(source),
        )
        .expect("ingest");
    let santi_core::IngestOutcome::Rejected { .. } = outcome else {
        panic!("expected over-budget rejection");
    };

    let strand = service
        .list_strands()
        .expect("strands")
        .into_iter()
        .find(|strand| strand.external_label.as_deref() == Some("test:no-payload-audit"))
        .expect("labeled strand");
    let incident = service
        .strand_errors(&strand.id, 10)
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
        input_budget_bytes: 100_000,
    });
    let service = service_with_budget(&temp, provider.clone());
    let strand = service.create_strand().expect("create strand").strand;
    let response = service
        .send_strand(
            &strand.id,
            SendStrandRequest {
                content: vec![MessagePart::Text {
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
    assert_eq!(runtime.errors[0].status, santi_core::IncidentStatus::Active);
    assert_eq!(
        runtime.errors[0].context["reason"],
        "provider_request_exceeds_budget"
    );
    assert!(
        runtime
            .messages
            .iter()
            .all(|message| message.message.message_kind != MessageKind::SantiSystem),
        "budget preflight must not project a model-visible notice"
    );

    let start_message_id = runtime
        .messages
        .iter()
        .find(|message| message.content_text == "please run the large tool")
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
            serde_json::to_string(&MessageContent::text("deliver after recovery"))
                .expect("serialize inbox content"),
            "2026-07-10T00:00:00Z",
        ],
    )
    .expect("seed pre-incident accepted input");
    drop(conn);
    let store = SantiStore::open(&db).expect("open store directly");
    let boundary = store
        .append_message(Draft {
            strand: &strand.id,
            actor: ActorType::Soul,
            id: store.default_soul_id(),
            content: MessageContent::text("manual compact boundary"),
            state: MessageState::Fixed,
            intake: MessageIntake::Record,
        })
        .expect("append manual boundary")
        .strand_message;

    let compact = service
        .compact_exec(
            &strand.id,
            santi_core::CompactExecRequest {
                from_message_id: Some(start_message_id),
                to_message_id: Some(boundary.message.id),
                from_seq: None,
                to_seq: None,
                summary: "Large tool exchange collapsed after context-budget incident.".to_string(),
                capsule: None,
                dry_run: false,
            },
        )
        .expect("compact should resolve incident when estimate is under budget");
    assert!(compact.active_incident_resolved);

    let after = Probe::new(&service).any_completed(&strand.id).await;
    assert_eq!(after.errors.len(), 1);
    assert_eq!(after.errors[0].status, santi_core::IncidentStatus::Resolved);
    assert_eq!(after.errors[0].revision, 2);
    assert_eq!(after.errors[0].resolved_by.as_deref(), Some("compact_exec"));
    assert!(
        after
            .messages
            .iter()
            .any(|message| message.content_text == "deliver after recovery"),
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
