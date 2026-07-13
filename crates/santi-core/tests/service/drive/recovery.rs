use super::super::support::*;
use rusqlite::Connection;

#[tokio::test]
async fn failed_receipt_redrives() {
    let temp = tempfile::tempdir().expect("temp dir");
    let provider = Arc::new(FakeProvider {
        fail_for_requests: Some(1),
        ..FakeProvider::default()
    });
    let service = SantiService::open(
        SantiServiceConfig {
            database_path: temp.path().join("santi.sqlite").display().to_string(),
            runtime_root: temp.path().join("runtime").display().to_string(),
            execution_root: temp.path().join("execution").display().to_string(),
            bind_addr: Some("127.0.0.1:0".to_string()),
        },
        provider.clone(),
    )
    .expect("open service");
    let strand = service.create_strand().expect("create strand").strand;
    let failed = service
        .send_strand(
            &strand.id,
            SendStrandRequest {
                content: vec![MessagePart::Text {
                    text: "one durable obligation".to_string(),
                }],
            },
        )
        .await
        .expect("send strand");
    wait_for_failed_turn(&service, &strand.id, &accepted_turn(&failed).id).await;

    let driven = service
        .drive_strand(&strand.id)
        .expect("explicit failed-receipt redrive");
    assert_eq!(driven.state, santi_core::DriveStrandState::Started);
    let recovered_turn = driven.turn.expect("recovery turn");
    let runtime = wait_for_completed_turn(&service, &strand.id, &recovered_turn.id).await;

    assert_eq!(provider.requests.lock().unwrap().len(), 2);
    assert_eq!(count_messages(&runtime, "one durable obligation"), 1);
    let receipt = service
        .receipt_status(&failed.receipt.inbox_id)
        .expect("receipt query")
        .expect("receipt");
    assert_eq!(receipt.state, santi_core::ReceiptState::Completed);
    assert_eq!(
        receipt
            .transitions
            .iter()
            .map(|transition| transition.state.clone())
            .collect::<Vec<_>>(),
        vec![
            santi_core::ReceiptState::Accepted,
            santi_core::ReceiptState::Driving,
            santi_core::ReceiptState::TurnFailed,
            santi_core::ReceiptState::Driving,
            santi_core::ReceiptState::Completed,
        ]
    );
}

#[tokio::test]
async fn drive_failure_recovers() {
    let temp = tempfile::tempdir().expect("temp dir");
    let database_path = temp.path().join("santi.sqlite");
    let service = SantiService::open(
        SantiServiceConfig {
            database_path: database_path.display().to_string(),
            runtime_root: temp.path().join("runtime").display().to_string(),
            execution_root: temp.path().join("execution").display().to_string(),
            bind_addr: Some("127.0.0.1:0".to_string()),
        },
        Arc::new(FakeProvider::default()),
    )
    .expect("open service");
    let strand = service.create_strand().expect("create strand").strand;
    let conn = Connection::open(&database_path).expect("open sqlite");
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
        .send_strand(
            &strand.id,
            SendStrandRequest {
                content: vec![MessagePart::Text {
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
    assert_eq!(warning.context["inbox_id"], accepted.receipt.inbox_id);
    assert_eq!(warning.context["recovery"]["resend"], false);
    let receipt_id = accepted.receipt.inbox_id.clone();
    let receipt = service
        .receipt_status(&receipt_id)
        .expect("receipt status")
        .expect("receipt");
    assert_eq!(receipt.state, santi_core::ReceiptState::Accepted);
    assert!(service.is_drive_degraded());

    let runtime = service
        .runtime_snapshot(&strand.id)
        .expect("runtime")
        .expect("strand");
    assert!(runtime.messages.is_empty());
    assert!(runtime.turns.is_empty());
    assert_eq!(runtime.errors.len(), 1);
    let incident_id = runtime.errors[0].id.clone();
    assert_eq!(runtime.errors[0].status, santi_core::IncidentStatus::Active);
    assert_eq!(runtime.errors[0].occurrence_count, 1);
    assert_eq!(pending_count(&conn, &strand.id), 1);

    let rejected = service
        .send_strand(
            &strand.id,
            SendStrandRequest {
                content: vec![MessagePart::Text {
                    text: "must not enter inbox".to_string(),
                }],
            },
        )
        .await
        .expect_err("active drive incident should quick-fail later writes");
    assert_eq!(rejected.code, "runtime.strand.drive_failed");
    assert_eq!(rejected.incident_id.as_deref(), Some(incident_id.as_str()));
    assert_eq!(rejected.context["accepted_before_failure"], false);
    assert_eq!(pending_count(&conn, &strand.id), 1);

    conn.execute_batch("DROP TRIGGER force_turn_insert_failure;")
        .expect("remove failure trigger");
    let driven = service.drive_strand(&strand.id).expect("operator redrive");
    assert_eq!(driven.state, santi_core::DriveStrandState::Started);
    let turn = driven.turn.expect("redrive turn");
    let runtime = wait_for_completed_turn(&service, &strand.id, &turn.id).await;
    assert!(!service.is_drive_degraded());
    assert_eq!(pending_count(&conn, &strand.id), 0);
    assert_eq!(runtime.errors.len(), 1);
    assert_eq!(runtime.errors[0].id, incident_id);
    assert_eq!(
        runtime.errors[0].status,
        santi_core::IncidentStatus::Resolved
    );
    assert_eq!(runtime.errors[0].occurrence_count, 2);
    assert_eq!(runtime.errors[0].revision, 2);
    assert_eq!(
        runtime.errors[0].resolved_by.as_deref(),
        Some("strand.drive_started")
    );
    assert!(
        runtime
            .messages
            .iter()
            .any(|message| message.content_text == "accepted before drive failure")
    );
    let receipt = service
        .receipt_status(&receipt_id)
        .expect("receipt status")
        .expect("receipt");
    assert_eq!(receipt.state, santi_core::ReceiptState::Completed);
    assert_eq!(
        receipt
            .transitions
            .iter()
            .map(|transition| transition.state.clone())
            .collect::<Vec<_>>(),
        vec![
            santi_core::ReceiptState::Accepted,
            santi_core::ReceiptState::MechanicallyRecovered,
            santi_core::ReceiptState::Driving,
            santi_core::ReceiptState::Completed,
        ]
    );
    assert_eq!(
        receipt.transitions[1].incident_id.as_deref(),
        Some(incident_id.as_str())
    );
    assert!(
        runtime
            .messages
            .iter()
            .all(|message| message.content_text != "must not enter inbox")
    );
    let transitions: i64 = conn
        .query_row("SELECT COUNT(*) FROM error_transitions", [], |row| {
            row.get(0)
        })
        .expect("transition count");
    assert_eq!(transitions, 2);
}

#[tokio::test]
async fn cold_start_recovers() {
    let temp = tempfile::tempdir().expect("temp dir");
    let database_path = temp.path().join("santi.sqlite");
    let config = SantiServiceConfig {
        database_path: database_path.display().to_string(),
        runtime_root: temp.path().join("runtime").display().to_string(),
        execution_root: temp.path().join("execution").display().to_string(),
        bind_addr: Some("127.0.0.1:0".to_string()),
    };
    let service = SantiService::open(config.clone(), Arc::new(FakeProvider::default()))
        .expect("open service");
    let strand = service.create_strand().expect("create strand").strand;
    service.begin_shutdown();
    let accepted = service
        .send_strand(
            &strand.id,
            SendStrandRequest {
                content: vec![MessagePart::Text {
                    text: "pending across restart".to_string(),
                }],
            },
        )
        .await
        .expect("shutdown should retain the durable request");
    assert!(accepted.receipt.warning.is_none());
    assert!(accepted.turn.is_none());
    drop(service);

    let conn = Connection::open(&database_path).expect("open sqlite");
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

    let restarted = SantiService::open(config, Arc::new(FakeProvider::default()))
        .expect("open restarted service");
    restarted
        .resume_pending()
        .expect("strand-local drive failure should permit degraded startup");
    assert!(restarted.is_drive_degraded());
    assert_eq!(pending_count(&conn, &strand.id), 1);
    let runtime = restarted
        .runtime_snapshot(&strand.id)
        .expect("runtime")
        .expect("strand");
    assert_eq!(runtime.errors.len(), 1);
    assert_eq!(runtime.errors[0].code, "runtime.strand.drive_failed");
    assert_eq!(runtime.errors[0].context["accepted_before_failure"], false);
    assert_eq!(runtime.errors[0].context["pending_count"], 1);
    assert_eq!(runtime.errors[0].source.operation, "cold_start_resume");

    let rejected = restarted
        .send_strand(
            &strand.id,
            SendStrandRequest {
                content: vec![MessagePart::Text {
                    text: "must still quick-fail".to_string(),
                }],
            },
        )
        .await
        .expect_err("active cold-start incident should gate writes");
    assert_eq!(rejected.code, "runtime.strand.drive_failed");
    assert_eq!(pending_count(&conn, &strand.id), 1);

    conn.execute_batch("DROP TRIGGER force_cold_start_turn_failure;")
        .expect("remove failure trigger");
    let driven = restarted
        .drive_strand(&strand.id)
        .expect("operator redrive");
    let turn = driven.turn.expect("redrive turn");
    let runtime = wait_for_completed_turn(&restarted, &strand.id, &turn.id).await;
    assert!(!restarted.is_drive_degraded());
    assert_eq!(pending_count(&conn, &strand.id), 0);
    assert_eq!(
        runtime.errors[0].status,
        santi_core::IncidentStatus::Resolved
    );
    assert!(
        runtime
            .messages
            .iter()
            .any(|message| message.content_text == "pending across restart")
    );
}

fn pending_count(conn: &Connection, strand_id: &str) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM strand_inbox WHERE strand_id = ?1",
        [strand_id],
        |row| row.get(0),
    )
    .expect("pending count")
}
