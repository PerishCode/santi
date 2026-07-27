use super::*;
use santi_core::{message, strand};

#[tokio::test]
async fn recovers() {
    let temp = tempfile::tempdir().expect("temp dir");
    let database = temp.path().join("santi.sqlite");
    let service = Service::open(
        service::Config {
            database: database.display().to_string(),
            runtime: temp.path().join("runtime").display().to_string(),
            execution: temp.path().join("execution").display().to_string(),
            bind: Some("127.0.0.1:0".to_string()),
            constitution: None,
        },
        Arc::new(FakeProvider::default()),
    )
    .expect("open service");
    let strand = service.weave().expect("create strand").strand;
    let conn = Connection::open(&database).expect("open sqlite");
    conn.execute_batch(
        r#"
        CREATE TRIGGER force_turn_insert_failure
        BEFORE INSERT ON turns
        BEGIN
          SELECT RAISE(ABORT, 'forced turn insert failure');
        END;
        "#,
    )
    .expect("install failure trigger");

    let accepted = service
        .send(
            &strand.id,
            strand::Post {
                content: vec![message::Part::Text {
                    text: "accepted before drive failure".to_string(),
                }],
            },
        )
        .await
        .expect("durable enqueue should remain accepted");
    assert!(accepted.turn.is_none());
    let warning = accepted.receipt.warning.as_deref().expect("drive warning");
    assert_eq!(warning.code, "runtime.strand.drive_failed");
    assert_eq!(warning.context["accepted_before_failure"], true);
    assert_eq!(warning.context["inbox"], accepted.receipt.inbox);
    assert_eq!(warning.context["recovery"]["resend"], false);
    let receipt_id = accepted.receipt.inbox.clone();
    let receipt = service
        .receipt(&receipt_id)
        .expect("receipt status")
        .expect("receipt");
    assert_eq!(receipt.state, santi_core::receipt::State::Accepted);
    assert!(service.degraded());

    let runtime = service
        .snapshot(&strand.id)
        .expect("runtime")
        .expect("strand");
    assert!(runtime.messages.is_empty());
    assert!(runtime.turns.is_empty());
    assert_eq!(runtime.errors.len(), 1);
    let incident = runtime.errors[0].id.clone();
    assert_eq!(runtime.errors[0].status, santi_core::Status::Active);
    assert_eq!(runtime.errors[0].occurrences, 1);
    assert_eq!(pending(&conn, &strand.id), 1);

    let rejected = service
        .send(
            &strand.id,
            strand::Post {
                content: vec![message::Part::Text {
                    text: "must not enter inbox".to_string(),
                }],
            },
        )
        .await
        .expect_err("active drive incident should quick-fail later writes");
    assert_eq!(rejected.code, "runtime.strand.drive_failed");
    assert_eq!(rejected.incident.as_deref(), Some(incident.as_str()));
    assert_eq!(rejected.context["accepted_before_failure"], false);
    assert_eq!(pending(&conn, &strand.id), 1);

    conn.execute_batch("DROP TRIGGER force_turn_insert_failure;")
        .expect("remove failure trigger");
    let driven = service.drive(&strand.id).expect("operator redrive");
    assert_eq!(driven.state, santi_core::drive::State::Started);
    let turn = driven.turn.expect("redrive turn");
    let runtime = Probe::new(&service)
        .completed_turn(&strand.id, &turn.id)
        .await;
    assert!(!service.degraded());
    assert_eq!(pending(&conn, &strand.id), 0);
    assert_eq!(runtime.errors.len(), 1);
    assert_eq!(runtime.errors[0].id, incident);
    assert_eq!(runtime.errors[0].status, santi_core::Status::Resolved);
    assert_eq!(runtime.errors[0].occurrences, 2);
    assert_eq!(runtime.errors[0].revision, 2);
    assert_eq!(
        runtime.errors[0].resolution.as_ref().unwrap().by.as_deref(),
        Some("strand.drive_started")
    );
    assert!(
        runtime
            .messages
            .iter()
            .any(|message| message.text == "accepted before drive failure")
    );
    let receipt = service
        .receipt(&receipt_id)
        .expect("receipt status")
        .expect("receipt");
    assert_eq!(receipt.state, santi_core::receipt::State::Completed);
    assert_eq!(
        receipt
            .transitions
            .iter()
            .map(|transition| transition.state.clone())
            .collect::<Vec<_>>(),
        vec![
            santi_core::receipt::State::Accepted,
            santi_core::receipt::State::Recovered,
            santi_core::receipt::State::Driving,
            santi_core::receipt::State::Completed,
        ]
    );
    assert_eq!(
        receipt.transitions[1].incident.as_deref(),
        Some(incident.as_str())
    );
    assert!(
        runtime
            .messages
            .iter()
            .all(|message| message.text != "must not enter inbox")
    );
    let transitions: i64 = conn
        .query_row("SELECT COUNT(*) FROM error_transitions", [], |row| {
            row.get(0)
        })
        .expect("transition count");
    assert_eq!(transitions, 2);
}
