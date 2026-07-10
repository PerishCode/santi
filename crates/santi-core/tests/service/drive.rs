use super::support::*;
use rusqlite::Connection;

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
    assert!(
        runtime
            .messages
            .iter()
            .all(|message| { message.content_text != "must not enter inbox" })
    );
    let transitions: i64 = conn
        .query_row("SELECT COUNT(*) FROM error_transitions", [], |row| {
            row.get(0)
        })
        .expect("transition count");
    assert_eq!(transitions, 2);
}

#[tokio::test]
async fn cold_start_drive_failure_degrades_and_recovers() {
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

#[tokio::test]
async fn reminder_no_repoke() {
    let temp = tempfile::tempdir().expect("temp dir");
    let provider = Arc::new(FakeProvider::default());
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
    let response = service
        .send_strand(
            &strand.id,
            SendStrandRequest {
                content: vec![MessagePart::Text {
                    text: "x".repeat(128 * 1024),
                }],
            },
        )
        .await
        .expect("send strand");

    let _runtime =
        wait_for_completed_turn(&service, &strand.id, &accepted_turn(&response).id).await;

    // The provider-input observation is drained only after completion. Wait for
    // the compact reminder Record to materialize, then give the completion
    // re-poke a chance to run. That re-poke must not start a second turn: the
    // reminder is a Record, not a new inbox Request.
    let _runtime =
        wait_for_message_containing(&service, &strand.id, "kind: compact_reminder").await;
    sleep(Duration::from_millis(100)).await;
    let runtime = service
        .runtime_snapshot(&strand.id)
        .expect("runtime snapshot")
        .expect("strand runtime");

    let requests = provider.requests.lock().unwrap();
    assert_eq!(
        requests.len(),
        1,
        "compact reminder completion path must not call the provider again"
    );
    assert_eq!(
        runtime.turns.len(),
        1,
        "compact reminder completion path must not create any duplicate turn row"
    );
    assert_eq!(
        runtime
            .turns
            .iter()
            .filter(|turn| turn.status == santi_core::TurnStatus::Completed)
            .count(),
        1,
        "compact reminder completion path must leave exactly one completed turn"
    );
    assert_eq!(
        runtime
            .messages
            .iter()
            .filter(|message| message.content_text.contains("kind: compact_reminder"))
            .count(),
        1,
        "large input should materialize exactly one compact reminder Record"
    );
}

#[tokio::test]
async fn concurrent_request_follows() {
    let temp = tempfile::tempdir().expect("temp dir");
    let provider = Arc::new(GatedFirstProvider::new());
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
    let first = service
        .send_strand(
            &strand.id,
            SendStrandRequest {
                content: vec![MessagePart::Text {
                    text: "first request".to_string(),
                }],
            },
        )
        .await
        .expect("send first request");
    let first_turn_id = accepted_turn(&first).id.clone();
    assert_eq!(
        first
            .user_message
            .as_ref()
            .expect("first send drove synchronously")
            .content_text,
        "first request"
    );

    provider.wait_for_first_request().await;

    let second = service
        .send_strand(
            &strand.id,
            SendStrandRequest {
                content: vec![MessagePart::Text {
                    text: "second request".to_string(),
                }],
            },
        )
        .await
        .expect("send second request while first is running");
    assert_eq!(
        accepted_turn(&second).id,
        first_turn_id,
        "a send during a running turn should report the turn it coalesced into"
    );
    assert!(
        second.user_message.is_none(),
        "coalesced send is still in the inbox, not yet a timeline message"
    );

    let running = service
        .runtime_snapshot(&strand.id)
        .expect("runtime snapshot")
        .expect("strand runtime");
    assert_eq!(running.turns.len(), 1);
    assert_eq!(running.turns[0].status, santi_core::TurnStatus::Running);
    assert_eq!(count_messages(&running, "first request"), 1);
    assert_eq!(count_messages(&running, "second request"), 0);
    assert_eq!(provider.requests.lock().unwrap().len(), 1);

    provider.release_first_request();
    let runtime = wait_completed_count(&service, &strand.id, 2).await;
    let requests = provider.requests.lock().unwrap();
    assert_eq!(
        requests.len(),
        2,
        "one queued real request should drive exactly one follow-on provider call"
    );
    assert!(
        provider_messages(&requests[0]).contains(&("user", "first request")),
        "first provider call should contain the original request"
    );
    assert!(
        !provider_messages(&requests[0]).contains(&("user", "second request")),
        "coalesced request must not leak into the already-built first provider input"
    );
    let second_input = provider_messages(&requests[1]);
    assert!(
        second_input.contains(&("user", "first request")),
        "follow-on provider call should replay the prior request"
    );
    assert!(
        second_input.contains(&("assistant", "provider response 1")),
        "follow-on provider call should replay the first assistant response"
    );
    assert!(
        second_input.contains(&("user", "second request")),
        "follow-on provider call should include the coalesced real request"
    );
    drop(requests);

    assert_eq!(runtime.turns.len(), 2);
    assert!(
        runtime
            .turns
            .iter()
            .all(|turn| turn.status == santi_core::TurnStatus::Completed)
    );
    assert_eq!(count_messages(&runtime, "first request"), 1);
    assert_eq!(count_messages(&runtime, "second request"), 1);
    assert_eq!(count_messages(&runtime, "provider response 1"), 1);
    assert_eq!(count_messages(&runtime, "provider response 2"), 1);
}

#[tokio::test]
async fn drain_preserves_provenance() {
    let temp = tempfile::tempdir().expect("temp dir");
    let provider = Arc::new(GatedFirstProvider::new());
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
    let first = service
        .send_strand(
            &strand.id,
            SendStrandRequest {
                content: vec![MessagePart::Text {
                    text: "first request".to_string(),
                }],
            },
        )
        .await
        .expect("send first request");
    let first_message_id = first
        .user_message
        .as_ref()
        .expect("first send drains immediately")
        .message
        .id
        .clone();

    provider.wait_for_first_request().await;

    let second = service
        .send_strand(
            &strand.id,
            SendStrandRequest {
                content: vec![MessagePart::Text {
                    text: "second request".to_string(),
                }],
            },
        )
        .await
        .expect("send second request while first runs");
    assert!(second.user_message.is_none());

    provider.release_first_request();
    let runtime = wait_completed_count(&service, &strand.id, 2).await;

    let second_message = runtime
        .messages
        .iter()
        .find(|message| message.content_text == "second request")
        .expect("second message drained after first turn");
    let second_event = runtime
        .message_events
        .iter()
        .find(|event| event.message_id == second_message.message.id)
        .expect("second message drain event");

    assert_eq!(second_event.payload["kind"], "inbox_drain");
    assert_eq!(
        second_event.payload["message_id"],
        second_message.message.id
    );
    assert_eq!(
        second_event.payload["drained_at"],
        second_message.message.created_at
    );
    assert_eq!(second_event.created_at, second_message.message.created_at);
    assert_eq!(
        second_event.payload["source"]["type"], "strand_send",
        "direct sends should carry caller/source shape"
    );
    assert_eq!(second_event.payload["source"]["ref"], strand.id);

    let follow_on_turn = runtime
        .turns
        .iter()
        .find(|turn| turn.id != accepted_turn(&first).id)
        .expect("follow-on turn");
    assert_eq!(
        second_event.payload["committing_turn_id"], follow_on_turn.id,
        "the drain event should name the turn that committed the pending request"
    );

    let enqueued_at = second_event.payload["enqueued_at"]
        .as_str()
        .expect("enqueued_at string");
    assert!(
        enqueued_at <= second_message.message.created_at.as_str(),
        "enqueue time should not be later than drain/message time"
    );

    let first_event = runtime
        .message_events
        .iter()
        .find(|event| event.message_id == first_message_id)
        .expect("first message drain event");
    assert_ne!(
        first_event.payload["inbox_id"], second_event.payload["inbox_id"],
        "each inbound request should keep its own inbox id provenance"
    );
}
