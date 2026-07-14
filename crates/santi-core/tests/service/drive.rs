#[path = "drive/recovery.rs"]
mod recovery;

use super::support::*;
use santi_core::service::{self, Service};

#[tokio::test]
async fn reminder_no_repoke() {
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

    let _runtime = Probe::new(&service)
        .completed_turn(&strand.id, &accepted_turn(&response).id)
        .await;

    let _runtime = Probe::new(&service)
        .message_containing(&strand.id, "kind: compact_reminder")
        .await;
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
    let first_receipt = service
        .receipt_status(&first.receipt.inbox_id)
        .expect("first receipt query")
        .expect("first receipt");
    assert_eq!(first_receipt.state, santi_core::ReceiptState::Driving);

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
    let second_receipt = service
        .receipt_status(&second.receipt.inbox_id)
        .expect("second receipt query")
        .expect("second receipt");
    assert_eq!(second_receipt.state, santi_core::ReceiptState::Accepted);

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
    let runtime = Probe::new(&service).completed_count(&strand.id, 2).await;
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

    for inbox_id in [&first.receipt.inbox_id, &second.receipt.inbox_id] {
        let receipt = service
            .receipt_status(inbox_id)
            .expect("receipt query")
            .expect("receipt");
        assert_eq!(receipt.state, santi_core::ReceiptState::Completed);
    }

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
    let runtime = Probe::new(&service).completed_count(&strand.id, 2).await;

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
