use super::*;

#[tokio::test]
async fn shutdown_pauses_consumption() {
    let temp = tempfile::tempdir().expect("temp dir");
    let config = service::Config {
        database_path: temp.path().join("santi.sqlite").display().to_string(),
        runtime_root: temp.path().join("runtime").display().to_string(),
        execution_root: temp.path().join("execution").display().to_string(),
        bind_addr: Some("127.0.0.1:0".to_string()),
        constitution_path: None,
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
            constitution_path: None,
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
