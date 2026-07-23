use super::*;
use santi_core::{message, receipt, turn};

#[tokio::test]
async fn preserves_aborted_output() {
    let temp = tempfile::tempdir().expect("temp dir");
    let provider = Arc::new(FailureProvider {
        stream_error_after_text: Some("provider stream aborted".to_string()),
        ..FailureProvider::default()
    });
    let service = open_service(&temp, provider.clone());
    let strand = service.weave().expect("create strand").strand;
    let response = send_text(&service, &strand.id, "trigger stream failure").await;

    let runtime = wait_for_aborted_output(&service, &strand.id, &turn(&response).id).await;
    let partial_message = runtime
        .messages
        .iter()
        .find(|message| {
            message.message.role == message::Role::Soul
                && message.message.state == message::State::Aborted
        })
        .expect("aborted partial assistant message");
    assert_eq!(partial_message.text, "partial runtime output");
    assert_no_failure_projection(&runtime);
    assert_eq!(runtime.errors.len(), 1);
    assert_eq!(runtime.errors[0].first.source.operation, "turn.stream");
    assert_eq!(runtime.errors[0].first.context["stage"], "stream");

    let retry = send_text(&service, &strand.id, "continue with preserved partial").await;
    wait_for_turn(&service, &strand.id, &turn(&retry).id, turn::Status::Failed).await;

    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].input.iter().any(|message| {
        as_text(message).is_some_and(|(role, content)| {
            role == "assistant" && content == "partial runtime output"
        })
    }));
    assert!(requests[1].input.iter().all(|message| {
        as_text(message).is_none_or(|(_, content)| !content.contains("kind: turn_failed"))
    }));
}

#[tokio::test]
async fn classifies_response_failure() {
    let temp = tempfile::tempdir().expect("temp dir");
    let provider = Arc::new(FailureProvider {
        response_failure: Some("provider rejected response".to_string()),
        ..FailureProvider::default()
    });
    let service = open_service(&temp, provider);
    let strand = service.weave().expect("create strand").strand;
    let response = send_text(&service, &strand.id, "trigger response failure").await;

    let runtime = wait_for_turn(
        &service,
        &strand.id,
        &turn(&response).id,
        turn::Status::Failed,
    )
    .await;
    assert_no_failure_projection(&runtime);
    assert_eq!(runtime.errors.len(), 1);
    assert_eq!(runtime.errors[0].first.source.operation, "turn.response");
    assert_eq!(runtime.errors[0].first.context["stage"], "response");
}

#[tokio::test]
async fn success_resolves_incident() {
    let temp = tempfile::tempdir().expect("temp dir");
    let provider = Arc::new(FailureProvider {
        fail_with: Some("temporary provider outage".to_string()),
        fail_for_requests: Some(1),
        ..FailureProvider::default()
    });
    let service = open_service(&temp, provider.clone());
    let strand = service.weave().expect("create strand").strand;
    let failed = send_text(&service, &strand.id, "first attempt").await;
    let before = wait_for_turn(
        &service,
        &strand.id,
        &turn(&failed).id,
        turn::Status::Failed,
    )
    .await;
    let held = before.errors[0].id.clone();

    let recovered = send_text(&service, &strand.id, "retry after recovery").await;
    let after = wait_for_turn(
        &service,
        &strand.id,
        &turn(&recovered).id,
        turn::Status::Completed,
    )
    .await;

    assert_no_failure_projection(&after);
    assert_eq!(provider.requests.lock().unwrap().len(), 2);
    assert_eq!(after.errors.len(), 1);
    let incident = &after.errors[0];
    assert_eq!(incident.id, held);
    assert_eq!(incident.status, santi_core::Status::Resolved);
    assert_eq!(incident.occurrences, 1);
    assert_eq!(incident.revision, 2);
    assert_eq!(
        incident.resolution.as_ref().unwrap().by.as_deref(),
        Some("provider.turn_succeeded")
    );
    assert_eq!(incident.latest.context["turn"], turn(&recovered).id);
    assert_eq!(incident.latest.context["provider"], "fake-provider");
    assert_eq!(incident.latest.context["model"], "fake-model");
    assert_eq!(
        transition_count(&temp),
        2,
        "only open and resolve are lifecycle transitions"
    );
}

#[tokio::test]
async fn runtime_receipt_recovers() {
    let temp = tempfile::tempdir().expect("temp dir");
    let provider = Arc::new(FailureProvider {
        started: true,
        ..FailureProvider::default()
    });
    let service = open_service(&temp, provider.clone());
    let strand = service.weave().expect("create strand").strand;
    let conn = Connection::open(temp.path().join("santi.sqlite")).expect("open sqlite");
    conn.execute_batch(
        r#"
        CREATE TRIGGER force_thinking_persistence_failure
        BEFORE INSERT ON thinking_spans
        BEGIN
          SELECT RAISE(ABORT, 'forced thinking persistence failure');
        END;
        "#,
    )
    .expect("install trigger");

    let failed = send_text(&service, &strand.id, "first obligation").await;
    let runtime = wait_for_turn(
        &service,
        &strand.id,
        &turn(&failed).id,
        turn::Status::Failed,
    )
    .await;
    assert_eq!(runtime.errors.len(), 1);
    let incident = &runtime.errors[0];
    assert_eq!(incident.code, "runtime.turn.failed");
    assert_eq!(incident.first.source.component, "santi-core");
    assert_eq!(incident.first.source.operation, "turn.thinking_persistence");
    assert_eq!(
        incident.first.context["operation"],
        "turn.thinking_persistence"
    );
    assert!(
        runtime
            .turns
            .iter()
            .find(|candidate| candidate.id == turn(&failed).id)
            .and_then(|turn| turn.error.as_deref())
            .is_some_and(|detail| detail.contains("forced thinking persistence failure"))
    );
    assert!(
        runtime
            .errors
            .iter()
            .all(|error| error.code != "provider.turn.failed")
    );
    let failed_receipt = service
        .receipt(&failed.receipt.inbox)
        .expect("receipt query")
        .expect("receipt");
    assert_eq!(failed_receipt.state, receipt::State::Failed);
    assert_eq!(
        failed_receipt
            .transitions
            .last()
            .and_then(|event| event.incident.as_deref()),
        Some(incident.id.as_str())
    );

    conn.execute_batch("DROP TRIGGER force_thinking_persistence_failure;")
        .expect("remove trigger");
    let successor = send_text(&service, &strand.id, "successor attempt").await;
    let runtime = wait_for_turn(
        &service,
        &strand.id,
        &turn(&successor).id,
        turn::Status::Completed,
    )
    .await;
    assert_eq!(runtime.errors[0].status, santi_core::Status::Resolved);
    assert_eq!(
        runtime.errors[0].resolution.as_ref().unwrap().by.as_deref(),
        Some("runtime.turn_succeeded")
    );
    let recovered_receipt = service
        .receipt(&failed.receipt.inbox)
        .expect("receipt query")
        .expect("receipt");
    assert_eq!(recovered_receipt.state, receipt::State::Completed);
    assert_eq!(
        recovered_receipt
            .transitions
            .iter()
            .map(|event| event.state.clone())
            .collect::<Vec<_>>(),
        vec![
            receipt::State::Accepted,
            receipt::State::Driving,
            receipt::State::Failed,
            receipt::State::Driving,
            receipt::State::Completed,
        ]
    );
    assert_eq!(provider.requests.lock().unwrap().len(), 2);
}
