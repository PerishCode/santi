use super::super::support::*;
use super::service_with_budget;

#[tokio::test]
async fn compact_redrives_receipt() {
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
                    text: "one compact-recoverable obligation".to_string(),
                }],
            },
        )
        .await
        .expect("send strand");
    let runtime = wait_for_failed_turn(&service, &strand.id, &accepted_turn(&response).id).await;
    let receipt = service
        .receipt_status(&response.receipt.inbox_id)
        .expect("receipt query")
        .expect("receipt");
    assert_eq!(receipt.state, santi_core::ReceiptState::TurnFailed);

    let start_message_id = runtime
        .messages
        .iter()
        .find(|message| message.content_text == "one compact-recoverable obligation")
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

    let compact = service
        .compact_exec(
            &strand.id,
            santi_core::CompactExecRequest {
                from_message_id: Some(start_message_id),
                to_message_id: Some(boundary.message.id),
                from_seq: None,
                to_seq: None,
                summary: "Failed tool exchange compacted for explicit recovery.".to_string(),
                capsule: None,
                dry_run: false,
            },
        )
        .expect("compact should resolve and redrive");
    assert!(compact.active_incident_resolved);

    let runtime = wait_any_completed(&service, &strand.id).await;
    assert_eq!(provider.requests.lock().unwrap().len(), 2);
    assert_eq!(runtime.effects.len(), 1, "effect must not replay");
    assert_eq!(runtime.effects[0].state, EffectState::Confirmed);
    assert_eq!(
        runtime.errors[0].status,
        santi_core::IncidentStatus::Resolved
    );
    let receipt = service
        .receipt_status(&response.receipt.inbox_id)
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
