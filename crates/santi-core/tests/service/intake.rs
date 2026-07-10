use super::support::*;

#[tokio::test]
async fn external_ingest_turn() {
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

    let soul_id = service.list_souls().expect("list souls")[0].id.clone();
    let label = "github:ops:issue:PerishCode/santi#42";
    let santi_core::IngestOutcome::Accepted { receipt } = service
        .ingest_external_event(&soul_id, label, "an external request arrived".to_string())
        .expect("ingest event")
    else {
        panic!("expected accepted");
    };
    let strand_id = receipt.strand_id;

    // The webhook event is a REQUEST → it wakes the soul on a label-anchored
    // strand. Wait for the system-triggered turn to complete.
    let runtime = wait_any_completed(&service, &strand_id).await;
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

    // A second event on the same label coalesces onto the same strand, not a new one.
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

    // A doorbell is a runtime-authored santi_system fact, not user speech — it
    // reaches the provider as a system-role message (see message_to_provider_item).
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

/// Boot recovery drains the inbox: content that an adaptor durably enqueued
/// but that never got drained before a crash (nobody called `ingest`'s poke —
/// simulated here by writing straight to the store, bypassing the service)
/// still drives a turn once a fresh service opens against the same db and
/// calls `resume_pending`.
#[tokio::test]
async fn boot_drains_inbox() {
    let temp = tempfile::tempdir().expect("temp dir");
    let config = SantiServiceConfig {
        database_path: temp.path().join("santi.sqlite").display().to_string(),
        runtime_root: temp.path().join("runtime").display().to_string(),
        execution_root: temp.path().join("execution").display().to_string(),
        bind_addr: Some("127.0.0.1:0".to_string()),
    };
    let provider = Arc::new(FakeProvider::default());

    let strand_id = {
        let service = SantiService::open(config.clone(), provider.clone()).expect("open service");
        service.create_strand().expect("create strand").strand.id
    };

    // Simulate an adaptor that enqueued content and then the process crashed
    // before any poke ever drained it: write directly to the inbox, bypassing
    // SantiService::ingest/send_strand entirely.
    let store = SantiStore::open(&config.database_path).expect("open store directly");
    store
        .enqueue_inbox(
            &strand_id,
            MessageKind::Text,
            MessageContent::text("stranded before the crash"),
        )
        .expect("enqueue inbox");
    drop(store);

    // A fresh service against the SAME db, as after a restart.
    let service = SantiService::open(config, provider.clone()).expect("reopen service");
    service.resume_pending().expect("resume pending");

    let runtime = wait_any_completed(&service, &strand_id).await;
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

/// Graceful shutdown pauses inbox CONSUMPTION (no new turns start) while ingest
/// keeps PRODUCING durably; a later fresh boot then drains what queued up. This
/// is the enabling behavior for self-upgrade: quiesce → stop → swap → start →
/// boot recovery wakes the soul on whatever queued during the window (PHASE-07).
#[tokio::test]
async fn shutdown_pauses_consumption() {
    let temp = tempfile::tempdir().expect("temp dir");
    let config = SantiServiceConfig {
        database_path: temp.path().join("santi.sqlite").display().to_string(),
        runtime_root: temp.path().join("runtime").display().to_string(),
        execution_root: temp.path().join("execution").display().to_string(),
        bind_addr: Some("127.0.0.1:0".to_string()),
    };
    let provider = Arc::new(FakeProvider::default());

    // A quiescing service: it accepts (durably enqueues) but starts NO turn.
    let strand_id = {
        let service = SantiService::open(config.clone(), provider.clone()).expect("open service");
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

    // Consumption paused: no turn was started. Production intact: the record is
    // durably queued (exactly what boot recovery scans for).
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

    // A fresh service (not shutting down) drains the backlog on boot.
    let service = SantiService::open(config, provider.clone()).expect("reopen service");
    service.resume_pending().expect("resume pending");
    let runtime = wait_any_completed(&service, &strand_id).await;
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

    let default_soul = service.list_souls().expect("list souls")[0].id.clone();
    let secretary = service
        .create_soul(CreateSoulRequest {
            memory: Some("# I am the secretary".to_string()),
        })
        .expect("create soul");
    assert_ne!(secretary.id, default_soul);

    // `create_strand` (client-facing, no label) always binds the default soul —
    // multi-soul-per-strand is gone, so a non-default soul is reached only via a
    // label-anchored strand (e.g. ingest_external_event), not by picking a soul
    // at send time.
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

    // A label-anchored strand can be owned by a non-default soul (via ingest).
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

    // An unknown strand id is rejected cleanly, not a 500.
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
