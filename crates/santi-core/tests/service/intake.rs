use super::support::*;
use santi_core::service::{self, Service};

#[tokio::test]
async fn external_ingest_turn() {
    let temp = tempfile::tempdir().expect("temp dir");
    let provider = Arc::new(FakeProvider::default());
    let service = Service::open(
        service::Config {
            database_path: temp.path().join("santi.sqlite").display().to_string(),
            runtime_root: temp.path().join("runtime").display().to_string(),
            execution_root: temp.path().join("execution").display().to_string(),
            bind_addr: Some("127.0.0.1:0".to_string()),
        },
        provider.clone(),
    )
    .expect("open service");

    let soul_id = service.list_souls().expect("list souls")[0].id.clone();
    let label = "github:ops:issue:PerishCode/santi#42";
    let santi_core::IngestOutcome::Accepted { receipt } = service
        .ingest_external_event(&soul_id, label, "an external request arrived".to_string())
        .expect("ingest event")
    else {
        panic!("expected accepted");
    };
    let strand_id = receipt.strand_id;

    let runtime = Probe::new(&service).any_completed(&strand_id).await;
    assert!(
        runtime
            .turns
            .iter()
            .any(|turn| turn.trigger_type == santi_core::TurnTriggerType::System)
    );
    assert!(
        runtime
            .messages
            .iter()
            .any(|message| message.content_text == "an external request arrived")
    );
    assert!(
        runtime
            .messages
            .iter()
            .any(|message| message.content_text == "hi from runtime")
    );

    let santi_core::IngestOutcome::Accepted {
        receipt: receipt_again,
    } = service
        .ingest_external_event(&soul_id, label, "a follow-up arrived".to_string())
        .expect("ingest second event")
    else {
        panic!("expected accepted");
    };
    let strand_id_again = receipt_again.strand_id;
    assert_eq!(strand_id_again, strand_id);

    let requests = provider.requests.lock().unwrap();
    assert!(requests.iter().any(|request| {
        request.input.iter().any(|item| {
            matches!(
                item,
                ProviderItem::Message { role, content }
                    if role == "system" && content == "an external request arrived"
            )
        })
    }));
}

#[tokio::test]
async fn completion_delivers() {
    let temp = tempfile::tempdir().expect("temp dir");
    let service = Service::open(
        service::Config {
            database_path: temp.path().join("santi.sqlite").display().to_string(),
            runtime_root: temp.path().join("runtime").display().to_string(),
            execution_root: temp.path().join("execution").display().to_string(),
            bind_addr: Some("127.0.0.1:0".to_string()),
        },
        Arc::new(FakeProvider::default()),
    )
    .expect("open service");

    let outcome = service
        .im_send(santi_core::DEFAULT_SOUL_ID, "operator", "hello")
        .expect("send IM");
    let santi_core::IngestOutcome::Accepted { receipt } = outcome else {
        panic!("IM send rejected");
    };
    let runtime = Probe::new(&service).any_completed(&receipt.strand_id).await;
    let turn = runtime
        .turns
        .iter()
        .find(|turn| turn.status == santi_core::TurnStatus::Completed)
        .expect("completed turn");

    let entries = service.im_poll("operator", 0).expect("poll replies");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].content, "hi from runtime");
    assert_eq!(entries[0].turn_id.as_deref(), Some(turn.id.as_str()));
    assert!(entries[0].message_id.is_some());
    assert_eq!(
        entries[0].delivery_mode,
        Some(santi_core::ImDeliveryMode::Automatic)
    );

    let status = service
        .receipt_status(&receipt.inbox_id)
        .expect("query receipt")
        .expect("receipt");
    assert_eq!(status.state, santi_core::ReceiptState::Completed);
    assert_eq!(status.im_deliveries.len(), 1);
    assert_eq!(status.im_deliveries[0].id, entries[0].id);
    assert_eq!(status.im_deliveries[0].turn_id, turn.id);
    assert_eq!(
        status.im_deliveries[0].delivery_mode,
        santi_core::ImDeliveryMode::Automatic
    );
}

#[tokio::test]
async fn delivery_failure_rolls_back() {
    let temp = tempfile::tempdir().expect("temp dir");
    let database_path = temp.path().join("santi.sqlite");
    let service = Service::open(
        service::Config {
            database_path: database_path.display().to_string(),
            runtime_root: temp.path().join("runtime").display().to_string(),
            execution_root: temp.path().join("execution").display().to_string(),
            bind_addr: Some("127.0.0.1:0".to_string()),
        },
        Arc::new(FakeProvider::default()),
    )
    .expect("open service");
    let conn = rusqlite::Connection::open(&database_path).unwrap();
    conn.execute_batch(
        r#"
        CREATE TRIGGER reject_im_delivery
        BEFORE INSERT ON im_inbox
        BEGIN
          SELECT RAISE(ABORT, 'injected IM delivery failure');
        END;
        "#,
    )
    .unwrap();
    drop(conn);

    let outcome = service
        .im_send(santi_core::DEFAULT_SOUL_ID, "operator", "hello")
        .expect("send IM");
    let santi_core::IngestOutcome::Accepted { receipt } = outcome else {
        panic!("IM send rejected");
    };
    let mut failed = None;
    for _ in 0..100 {
        let status = service
            .receipt_status(&receipt.inbox_id)
            .expect("query receipt")
            .expect("receipt");
        if status.state == santi_core::ReceiptState::TurnFailed {
            failed = Some(status);
            break;
        }
        sleep(Duration::from_millis(20)).await;
    }
    let status = failed.expect("receipt should fail when delivery cannot commit");
    assert!(status.im_deliveries.is_empty());
    let runtime = service
        .runtime_snapshot(&receipt.strand_id)
        .unwrap()
        .expect("runtime");
    assert!(
        runtime
            .turns
            .iter()
            .all(|turn| turn.status != santi_core::TurnStatus::Completed)
    );
}

#[tokio::test]
async fn boot_drains_inbox() {
    let temp = tempfile::tempdir().expect("temp dir");
    let config = service::Config {
        database_path: temp.path().join("santi.sqlite").display().to_string(),
        runtime_root: temp.path().join("runtime").display().to_string(),
        execution_root: temp.path().join("execution").display().to_string(),
        bind_addr: Some("127.0.0.1:0".to_string()),
    };
    let provider = Arc::new(FakeProvider::default());

    let strand_id = {
        let service = Service::open(config.clone(), provider.clone()).expect("open service");
        service.create_strand().expect("create strand").strand.id
    };

    let store = SantiStore::open(&config.database_path).expect("open store directly");
    store
        .enqueue_inbox(
            &strand_id,
            MessageKind::Text,
            MessageContent::text("stranded before the crash"),
        )
        .expect("enqueue inbox");
    drop(store);

    let service = Service::open(config, provider.clone()).expect("reopen service");
    service.resume_pending().expect("resume pending");

    let runtime = Probe::new(&service).any_completed(&strand_id).await;
    assert!(
        runtime
            .messages
            .iter()
            .any(|message| message.content_text == "stranded before the crash")
    );
    assert!(
        runtime
            .messages
            .iter()
            .any(|message| message.content_text == "hi from runtime")
    );
}

#[tokio::test]
async fn shutdown_pauses_consumption() {
    let temp = tempfile::tempdir().expect("temp dir");
    let config = service::Config {
        database_path: temp.path().join("santi.sqlite").display().to_string(),
        runtime_root: temp.path().join("runtime").display().to_string(),
        execution_root: temp.path().join("execution").display().to_string(),
        bind_addr: Some("127.0.0.1:0".to_string()),
    };
    let provider = Arc::new(FakeProvider::default());

    let strand_id = {
        let service = Service::open(config.clone(), provider.clone()).expect("open service");
        service.begin_shutdown();
        assert!(service.is_shutting_down());
        let outcome = service
            .ingest_external_event(
                "soul_default",
                "shutdown:quiesce",
                "arrived while quiescing".to_string(),
            )
            .expect("ingest during shutdown");
        match outcome {
            santi_core::IngestOutcome::Accepted { receipt } => receipt.strand_id,
            other => panic!("expected accepted, got {other:?}"),
        }
    };

    let store = SantiStore::open(&config.database_path).expect("open store directly");
    assert_eq!(
        store.running_turn_count().expect("count"),
        0,
        "shutdown must not start a turn"
    );
    assert!(
        store
            .strands_with_pending_requests()
            .expect("pending")
            .contains(&strand_id),
        "the ingested record must still be durably queued"
    );
    drop(store);

    let service = Service::open(config, provider.clone()).expect("reopen service");
    service.resume_pending().expect("resume pending");
    let runtime = Probe::new(&service).any_completed(&strand_id).await;
    assert!(
        runtime
            .messages
            .iter()
            .any(|message| message.content_text == "arrived while quiescing")
    );
}

#[tokio::test]
async fn send_targets_soul() {
    let temp = tempfile::tempdir().expect("temp dir");
    let provider = Arc::new(FakeProvider::default());
    let service = Service::open(
        service::Config {
            database_path: temp.path().join("santi.sqlite").display().to_string(),
            runtime_root: temp.path().join("runtime").display().to_string(),
            execution_root: temp.path().join("execution").display().to_string(),
            bind_addr: Some("127.0.0.1:0".to_string()),
        },
        provider.clone(),
    )
    .expect("open service");

    let default_soul = service.list_souls().expect("list souls")[0].id.clone();
    let secretary = service
        .create_soul(CreateSoulRequest {
            memory: Some("# I am the secretary".to_string()),
        })
        .expect("create soul");
    assert_ne!(secretary.id, default_soul);

    let strand = service.create_strand().expect("create strand").strand;
    assert_eq!(strand.soul_id, default_soul);
    let response = service
        .send_strand(
            &strand.id,
            SendStrandRequest {
                content: vec![MessagePart::Text {
                    text: "for whoever".to_string(),
                }],
            },
        )
        .await
        .expect("send strand");
    assert_eq!(response.strand.soul_id, default_soul);

    let santi_core::IngestOutcome::Accepted {
        receipt: secretary_receipt,
    } = service
        .ingest_external_event(
            &secretary.id,
            "github:issue:1",
            "hello secretary".to_string(),
        )
        .expect("ingest event")
    else {
        panic!("expected accepted");
    };
    let secretary_strand_id = secretary_receipt.strand_id;
    let secretary_response = service
        .send_strand(
            &secretary_strand_id,
            SendStrandRequest {
                content: vec![MessagePart::Text {
                    text: "for the secretary".to_string(),
                }],
            },
        )
        .await
        .expect("send strand");
    assert_eq!(secretary_response.strand.soul_id, secretary.id);

    let error = service
        .send_strand(
            "ss_does_not_exist",
            SendStrandRequest {
                content: vec![MessagePart::Text {
                    text: "nobody home".to_string(),
                }],
            },
        )
        .await
        .expect_err("unknown strand should error");
    assert!(error.message.contains("strand not found"), "got: {error}");
}
