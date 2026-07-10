use super::support::*;

#[tokio::test]
async fn over_budget_send_blocks() {
    let temp = tempfile::tempdir().expect("temp dir");
    let provider = Arc::new(FakeProvider {
        input_budget_bytes: Some(1),
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
    let err = service
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

    assert!(err.contains("strand context is over budget"), "got: {err}");
    assert!(
        provider.requests.lock().unwrap().is_empty(),
        "provider must not receive an over-budget request"
    );
    let runtime = service
        .runtime_snapshot(&strand.id)
        .expect("runtime")
        .expect("strand");
    assert!(runtime.messages.is_empty(), "rejected send entered spine");
    assert!(runtime.turns.is_empty(), "rejected send started a turn");
    assert_eq!(runtime.blocks.len(), 1);
    assert_eq!(runtime.blocks[0].kind, "context_over_budget");
    assert_eq!(runtime.blocks[0].status, "active");
    assert_eq!(
        runtime.blocks[0].reason_code,
        "candidate_input_exceeds_budget"
    );
    assert_eq!(runtime.rejected_deliveries.len(), 1);
    assert_eq!(
        runtime.rejected_deliveries[0].reason_code,
        "candidate_input_exceeds_budget"
    );
    assert!(
        runtime.rejected_deliveries[0]
            .content_excerpt
            .contains("this should not enter")
    );
}

#[tokio::test]
async fn active_block_rejects_followup() {
    let temp = tempfile::tempdir().expect("temp dir");
    let provider = Arc::new(FakeProvider {
        input_budget_bytes: Some(1),
        ..FakeProvider::default()
    });
    let service = SantiService::open(
        SantiServiceConfig {
            database_path: temp.path().join("santi.sqlite").display().to_string(),
            runtime_root: temp.path().join("runtime").display().to_string(),
            execution_root: temp.path().join("execution").display().to_string(),
            bind_addr: Some("127.0.0.1:0".to_string()),
        },
        provider,
    )
    .expect("open service");

    let strand = service.create_strand().expect("create strand").strand;
    for text in ["first rejected", "second rejected"] {
        let err = service
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
        assert!(
            err.contains("context_over_budget") || err.contains("over budget"),
            "got: {err}"
        );
    }

    let runtime = service
        .runtime_snapshot(&strand.id)
        .expect("runtime")
        .expect("strand");
    assert!(runtime.messages.is_empty(), "blocked sends entered spine");
    assert!(runtime.turns.is_empty(), "blocked sends started turns");
    assert_eq!(runtime.blocks.len(), 1);
    assert_eq!(runtime.rejected_deliveries.len(), 2);
    assert!(
        runtime
            .rejected_deliveries
            .iter()
            .any(|delivery| delivery.reason_code == "context_over_budget_active")
    );
}

#[tokio::test]
async fn active_block_rejects_store() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db = temp.path().join("santi.sqlite");
    let provider = Arc::new(FakeProvider {
        input_budget_bytes: Some(1),
        ..FakeProvider::default()
    });
    let service = SantiService::open(
        SantiServiceConfig {
            database_path: db.display().to_string(),
            runtime_root: temp.path().join("runtime").display().to_string(),
            execution_root: temp.path().join("execution").display().to_string(),
            bind_addr: Some("127.0.0.1:0".to_string()),
        },
        provider,
    )
    .expect("open service");

    let strand = service.create_strand().expect("create strand").strand;
    service
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
    let santi_core::IngestOutcome::Rejected { reason } = outcome else {
        panic!("direct enqueue should be rejected by the active context block");
    };
    assert!(reason.contains("context_over_budget"), "got: {reason}");

    let runtime = service
        .runtime_snapshot(&strand.id)
        .expect("runtime")
        .expect("strand");
    assert!(
        runtime.messages.is_empty(),
        "blocked direct enqueue entered spine"
    );
    assert!(
        runtime.turns.is_empty(),
        "blocked direct enqueue started a turn"
    );
    assert_eq!(runtime.blocks.len(), 1);
    assert_eq!(runtime.rejected_deliveries.len(), 2);
    assert!(
        runtime
            .rejected_deliveries
            .iter()
            .any(|delivery| delivery.content_excerpt.contains("direct bypass"))
    );
}

#[tokio::test]
async fn pending_resume_rejects() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db = temp.path().join("santi.sqlite");
    let provider = Arc::new(FakeProvider {
        input_budget_bytes: Some(1),
        ..FakeProvider::default()
    });
    let service = SantiService::open(
        SantiServiceConfig {
            database_path: db.display().to_string(),
            runtime_root: temp.path().join("runtime").display().to_string(),
            execution_root: temp.path().join("execution").display().to_string(),
            bind_addr: Some("127.0.0.1:0".to_string()),
        },
        provider.clone(),
    )
    .expect("open service");

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
        panic!("offline source-less enqueue should be accepted before a block exists");
    };
    drop(store);

    service.resume_pending();

    let runtime = service
        .runtime_snapshot(&strand.id)
        .expect("runtime")
        .expect("strand");
    assert!(
        provider.requests.lock().unwrap().is_empty(),
        "provider must not receive an over-budget pending drain"
    );
    assert!(
        runtime.messages.is_empty(),
        "over-budget pending inbox entered spine"
    );
    assert!(
        runtime.turns.is_empty(),
        "over-budget pending started a turn"
    );
    assert_eq!(runtime.blocks.len(), 1);
    assert_eq!(
        runtime.blocks[0].reason_code,
        "pending_drain_would_exceed_budget"
    );
    assert_eq!(runtime.rejected_deliveries.len(), 1);
    assert_eq!(
        runtime.rejected_deliveries[0].reason_code,
        "pending_drain_would_exceed_budget"
    );
}

#[tokio::test]
async fn rejection_caps_audit() {
    let temp = tempfile::tempdir().expect("temp dir");
    let provider = Arc::new(FakeProvider {
        input_budget_bytes: Some(1),
        ..FakeProvider::default()
    });
    let service = SantiService::open(
        SantiServiceConfig {
            database_path: temp.path().join("santi.sqlite").display().to_string(),
            runtime_root: temp.path().join("runtime").display().to_string(),
            execution_root: temp.path().join("execution").display().to_string(),
            bind_addr: Some("127.0.0.1:0".to_string()),
        },
        provider,
    )
    .expect("open service");

    let soul_id = service.list_souls().expect("souls")[0].id.clone();
    let source = InboxSource::new("test").with_metadata(json!({
        "raw": "secret-ish webhook body ".repeat(600),
    }));
    let outcome = service
        .ingest_external_source(
            &soul_id,
            "test:rejected-audit-cap",
            "x".repeat(5_000),
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
        .find(|strand| strand.external_label.as_deref() == Some("test:rejected-audit-cap"))
        .expect("labeled strand");
    let runtime = service
        .runtime_snapshot(&strand.id)
        .expect("runtime")
        .expect("strand");
    let delivery = &runtime.rejected_deliveries[0];
    assert!(delivery.content_excerpt.len() <= 1024);
    assert!(delivery.content_excerpt.contains("[truncated]"));
    let metadata = delivery.source_metadata.as_ref().expect("source metadata");
    assert_eq!(
        metadata["schema"],
        "santi.rejected_source_metadata_truncated.v1"
    );
    assert_eq!(metadata["truncated"], true);
    assert!(metadata["original_bytes"].as_i64().expect("bytes") > 4096);
    assert!(
        !metadata.to_string().contains("secret-ish"),
        "large source metadata should not be stored verbatim"
    );
}

#[tokio::test]
async fn preflight_block_compact_clear() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db = temp.path().join("santi.sqlite");
    let provider = Arc::new(LargeToolCallProvider {
        requests: Arc::new(Mutex::new(Vec::new())),
        input_budget_bytes: 100_000,
    });
    let service = SantiService::open(
        SantiServiceConfig {
            database_path: db.display().to_string(),
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
                    text: "please run the large tool".to_string(),
                }],
            },
        )
        .await
        .expect("send strand");

    let runtime = wait_for_failed_turn(&service, &strand.id, &response.turn.id).await;
    assert_eq!(provider.requests.lock().unwrap().len(), 1);
    assert_eq!(runtime.blocks.len(), 1);
    assert_eq!(runtime.blocks[0].status, "active");
    assert_eq!(
        runtime.blocks[0].reason_code,
        "provider_request_exceeds_budget"
    );
    assert!(
        !runtime
            .messages
            .iter()
            .any(|message| message.content_text.contains("kind: turn_failed")),
        "context-budget preflight must not grow the spine with a failure notice"
    );
    assert!(
        runtime
            .messages
            .iter()
            .all(|message| message.message.message_kind != MessageKind::SantiSystem),
        "context-budget preflight must not materialize runtime santi_system notices"
    );

    let start_message_id = runtime
        .messages
        .iter()
        .find(|message| message.content_text == "please run the large tool")
        .expect("user message")
        .message
        .id
        .clone();
    let store = SantiStore::open(&db).expect("open store directly");
    let boundary = store
        .append_message(
            &strand.id,
            ActorType::Soul,
            store.default_soul_id(),
            MessageContent::text("manual compact boundary"),
            MessageState::Fixed,
            MessageIntake::Record,
        )
        .expect("append manual boundary")
        .strand_message;

    service
        .compact_exec(
            &strand.id,
            santi_core::CompactExecRequest {
                from_message_id: Some(start_message_id),
                to_message_id: Some(boundary.message.id),
                from_seq: None,
                to_seq: None,
                summary: "Large tool exchange collapsed after context-budget block.".to_string(),
                capsule: None,
                dry_run: false,
            },
        )
        .expect("compact should clear block when estimate is back under budget");

    let after = service
        .runtime_snapshot(&strand.id)
        .expect("runtime")
        .expect("strand");
    assert!(
        after.blocks.iter().all(|block| block.status != "active"),
        "compact should clear the active context block: {:?}",
        after.blocks
    );
    assert!(
        after.blocks.iter().any(|block| block.status == "cleared"),
        "cleared block should remain auditable"
    );
}
