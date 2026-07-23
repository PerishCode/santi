use super::support::*;
use rusqlite::Connection;
use santi_core::service::{self, Service};
use santi_core::{message, strand};

mod more;

mod memory_pressure;

mod recovery;

fn service_with_budget(temp: &tempfile::TempDir, provider: Arc<dyn Provider>) -> Service {
    Service::open(
        service::Config {
            database: temp.path().join("santi.sqlite").display().to_string(),
            runtime: temp.path().join("runtime").display().to_string(),
            execution: temp.path().join("execution").display().to_string(),
            bind: Some("127.0.0.1:0".to_string()),
            constitution: None,
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
        bytes: Some(1),
        ..FakeProvider::default()
    });
    let service = service_with_budget(&temp, provider.clone());
    let mut events = service.listen();
    let strand = service.weave().expect("create strand").strand;

    let error = service
        .send(
            &strand.id,
            strand::Post {
                content: vec![message::Part::Text {
                    text: "this should not enter the strand".to_string(),
                }],
            },
        )
        .await
        .expect_err("send should be rejected");

    assert_eq!(error.code, "context.budget.exceeded");
    let held = error.incident.expect("incident id");
    assert!(error.message.contains("strand context is over budget"));
    assert!(provider.requests.lock().unwrap().is_empty());
    let runtime = service
        .snapshot(&strand.id)
        .expect("runtime")
        .expect("strand");
    assert!(runtime.messages.is_empty(), "rejected send entered spine");
    assert!(runtime.turns.is_empty(), "rejected send started a turn");
    assert_eq!(runtime.errors.len(), 1);
    let incident = &runtime.errors[0];
    assert_eq!(incident.id, held);
    assert_eq!(incident.status, santi_core::Status::Active);
    assert_eq!(incident.occurrences, 1);
    assert_eq!(incident.revision, 1);
    assert_eq!(incident.first.source.operation, "ingest_admission");
    assert_eq!(
        incident.first.context["reason"],
        "candidate_input_exceeds_budget"
    );

    let transition = std::iter::from_fn(|| events.try_recv().ok())
        .find_map(|event| match event.payload {
            santi_core::stream::Payload::Transition { transition } => Some(transition),
            _ => None,
        })
        .expect("error transition event");
    assert_eq!(transition.incident, held);
    assert_eq!(transition.revision, 1);
    assert_eq!(transition.kind, santi_core::Kind::Opened);

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
    let runtime = temp.path().join("runtime");
    let memory_path = runtime.join("souls/soul_default/memory/MEMORY.md");
    fs::create_dir_all(memory_path.parent().expect("memory parent")).expect("create memory parent");
    fs::write(&memory_path, "m".repeat(9_000)).expect("write initial memory");
    let provider = Arc::new(FakeProvider {
        bytes: Some(20_000),
        ..FakeProvider::default()
    });
    let service = Service::open(
        service::Config {
            database: temp.path().join("santi.sqlite").display().to_string(),
            runtime: runtime.display().to_string(),
            execution: temp.path().join("execution").display().to_string(),
            bind: Some("127.0.0.1:0".to_string()),
            constitution: None,
        },
        provider.clone(),
    )
    .expect("open service");
    let strand = service.weave().expect("create strand").strand;

    let error = service
        .send(
            &strand.id,
            strand::Post {
                content: vec![message::Part::Text {
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
        .send(
            &strand.id,
            strand::Post {
                content: vec![message::Part::Text {
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
            .incident
            .is_none()
    );
    let incidents = service
        .strand_errors(&strand.id, 10)
        .expect("strand errors")
        .expect("strand");
    assert_eq!(incidents.len(), 1);
    assert_eq!(incidents[0].status, santi_core::Status::Resolved);
}

#[tokio::test]
async fn repeats_are_idempotent() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db = temp.path().join("santi.sqlite");
    let provider = Arc::new(FakeProvider {
        bytes: Some(1),
        ..FakeProvider::default()
    });
    let service = service_with_budget(&temp, provider);
    let strand = service.weave().expect("create strand").strand;

    let mut incident_ids = Vec::new();
    for text in ["first rejected", "second rejected", "third rejected"] {
        let error = service
            .send(
                &strand.id,
                strand::Post {
                    content: vec![message::Part::Text {
                        text: text.to_string(),
                    }],
                },
            )
            .await
            .expect_err("send should be rejected");
        incident_ids.push(error.incident.expect("incident id"));
    }
    assert!(incident_ids.windows(2).all(|ids| ids[0] == ids[1]));

    let runtime = service
        .snapshot(&strand.id)
        .expect("runtime")
        .expect("strand");
    assert!(runtime.messages.is_empty());
    assert!(runtime.turns.is_empty());
    assert_eq!(runtime.errors.len(), 1);
    assert_eq!(runtime.errors[0].occurrences, 3);
    assert_eq!(runtime.errors[0].revision, 1);
    assert_eq!(
        runtime.errors[0].latest.source.operation,
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

    let mut events = service.listen();
    let event = events.try_recv().expect("pending transition");
    assert!(matches!(
        event.payload,
        santi_core::stream::Payload::Transition { .. }
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
        bytes: Some(1),
        ..FakeProvider::default()
    });
    let service = service_with_budget(&temp, provider);
    let strand = service.weave().expect("create strand").strand;
    let first = service
        .send(
            &strand.id,
            strand::Post {
                content: vec![message::Part::Text {
                    text: "first rejected".to_string(),
                }],
            },
        )
        .await
        .expect_err("send should be rejected");

    let store = Store::open(&db).expect("open store directly");
    let outcome = store
        .enqueue_inbox(
            &strand.id,
            message::Kind::Text,
            message::Content::text("direct bypass attempt"),
        )
        .expect("direct enqueue");
    let santi_core::ingest::Outcome::Rejected { error } = outcome else {
        panic!("direct enqueue should be rejected by the active incident");
    };
    assert_eq!(error.incident, first.incident);
    let runtime = service
        .snapshot(&strand.id)
        .expect("runtime")
        .expect("strand");
    assert_eq!(runtime.errors[0].occurrences, 2);
    assert!(runtime.messages.is_empty());
}
