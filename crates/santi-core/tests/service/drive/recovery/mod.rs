use super::super::support::*;
use rusqlite::Connection;
use santi_core::service::{self, Service};
use santi_core::{message, strand};

mod more;

#[tokio::test]
async fn failed_receipt_redrives() {
    let temp = tempfile::tempdir().expect("temp dir");
    let provider = Arc::new(FakeProvider {
        fail_for_requests: Some(1),
        ..FakeProvider::default()
    });
    let service = Service::open(
        service::Config {
            database: temp.path().join("santi.sqlite").display().to_string(),
            runtime: temp.path().join("runtime").display().to_string(),
            execution: temp.path().join("execution").display().to_string(),
            bind: Some("127.0.0.1:0".to_string()),
            constitution: None,
        },
        provider.clone(),
    )
    .expect("open service");
    let strand = service.weave().expect("create strand").strand;
    let failed = service
        .send(
            &strand.id,
            strand::Post {
                content: vec![message::Part::Text {
                    text: "one durable obligation".to_string(),
                }],
            },
        )
        .await
        .expect("send strand");
    Probe::new(&service)
        .failed_turn(&strand.id, &accepted_turn(&failed).id)
        .await;

    let driven = service
        .drive_strand(&strand.id)
        .expect("explicit failed-receipt redrive");
    assert_eq!(driven.state, santi_core::drive::State::Started);
    let recovered_turn = driven.turn.expect("recovery turn");
    let runtime = Probe::new(&service)
        .completed_turn(&strand.id, &recovered_turn.id)
        .await;

    assert_eq!(provider.requests.lock().unwrap().len(), 2);
    assert_eq!(count_messages(&runtime, "one durable obligation"), 1);
    let receipt = service
        .receipt(&failed.receipt.inbox)
        .expect("receipt query")
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
            santi_core::receipt::State::Driving,
            santi_core::receipt::State::Failed,
            santi_core::receipt::State::Driving,
            santi_core::receipt::State::Completed,
        ]
    );
}

#[tokio::test]
async fn cold_start_recovers() {
    let temp = tempfile::tempdir().expect("temp dir");
    let database = temp.path().join("santi.sqlite");
    let config = service::Config {
        database: database.display().to_string(),
        runtime: temp.path().join("runtime").display().to_string(),
        execution: temp.path().join("execution").display().to_string(),
        bind: Some("127.0.0.1:0".to_string()),
        constitution: None,
    };
    let service =
        Service::open(config.clone(), Arc::new(FakeProvider::default())).expect("open service");
    let strand = service.weave().expect("create strand").strand;
    service.close();
    let accepted = service
        .send(
            &strand.id,
            strand::Post {
                content: vec![message::Part::Text {
                    text: "pending across restart".to_string(),
                }],
            },
        )
        .await
        .expect("shutdown should retain the durable request");
    assert!(accepted.receipt.warning.is_none());
    assert!(accepted.turn.is_none());
    drop(service);

    let conn = Connection::open(&database).expect("open sqlite");
    conn.execute_batch(
        r#"
        CREATE TRIGGER force_cold_start_turn_failure
        BEFORE INSERT ON turns
        BEGIN
          SELECT RAISE(ABORT, 'forced cold-start turn failure');
        END;
        "#,
    )
    .expect("install failure trigger");

    let restarted =
        Service::open(config, Arc::new(FakeProvider::default())).expect("open restarted service");
    restarted
        .resume()
        .expect("strand-local drive failure should permit degraded startup");
    assert!(restarted.degraded());
    assert_eq!(pending(&conn, &strand.id), 1);
    let runtime = restarted
        .snapshot(&strand.id)
        .expect("runtime")
        .expect("strand");
    assert_eq!(runtime.errors.len(), 1);
    assert_eq!(runtime.errors[0].code, "runtime.strand.drive_failed");
    assert_eq!(
        runtime.errors[0].first.context["accepted_before_failure"],
        false
    );
    assert_eq!(runtime.errors[0].first.context["pending_count"], 1);
    assert_eq!(
        runtime.errors[0].first.source.operation,
        "cold_start_resume"
    );

    let rejected = restarted
        .send(
            &strand.id,
            strand::Post {
                content: vec![message::Part::Text {
                    text: "must still quick-fail".to_string(),
                }],
            },
        )
        .await
        .expect_err("active cold-start incident should gate writes");
    assert_eq!(rejected.code, "runtime.strand.drive_failed");
    assert_eq!(pending(&conn, &strand.id), 1);

    conn.execute_batch("DROP TRIGGER force_cold_start_turn_failure;")
        .expect("remove failure trigger");
    let driven = restarted
        .drive_strand(&strand.id)
        .expect("operator redrive");
    let turn = driven.turn.expect("redrive turn");
    let runtime = Probe::new(&restarted)
        .completed_turn(&strand.id, &turn.id)
        .await;
    assert!(!restarted.degraded());
    assert_eq!(pending(&conn, &strand.id), 0);
    assert_eq!(runtime.errors[0].status, santi_core::Status::Resolved);
    assert!(
        runtime
            .messages
            .iter()
            .any(|message| message.text == "pending across restart")
    );
}

fn pending(conn: &Connection, strand: &str) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM strand_inbox WHERE strand_id = ?1",
        [strand],
        |row| row.get(0),
    )
    .expect("pending count")
}
