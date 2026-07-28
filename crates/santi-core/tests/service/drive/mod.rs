mod extra;
mod stop;

mod recovery;

use super::support::*;
use santi_core::service::{self, Service};
use santi_core::{message, strand};

#[tokio::test]
async fn reminder() {
    let temp = tempfile::tempdir().expect("temp dir");
    let provider = Arc::new(FakeProvider::default());
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
    let response = service
        .send(
            &strand.id,
            strand::Post {
                content: vec![message::Part::Text {
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
        .snapshot(&strand.id)
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
            .filter(|turn| turn.status == santi_core::turn::Status::Completed)
            .count(),
        1,
        "compact reminder completion path must leave exactly one completed turn"
    );
    assert_eq!(
        runtime
            .messages
            .iter()
            .filter(|message| message.text.contains("kind: compact_reminder"))
            .count(),
        1,
        "large input should materialize exactly one compact reminder Record"
    );
}

#[tokio::test]
async fn follows() {
    let temp = tempfile::tempdir().expect("temp dir");
    let provider = Arc::new(GatedFirstProvider::new());
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
    let first = service
        .send(
            &strand.id,
            strand::Post {
                content: vec![message::Part::Text {
                    text: "first request".to_string(),
                }],
            },
        )
        .await
        .expect("send first request");
    let first_turn_id = accepted_turn(&first).id.clone();
    assert_eq!(
        first
            .message
            .as_ref()
            .expect("first send drove synchronously")
            .text,
        "first request"
    );

    provider.wait_for_first_request().await;
    let first_receipt = service
        .receipt(&first.receipt.inbox)
        .expect("first receipt query")
        .expect("first receipt");
    assert_eq!(first_receipt.state, santi_core::receipt::State::Driving);

    let second = service
        .send(
            &strand.id,
            strand::Post {
                content: vec![message::Part::Text {
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
        second.message.is_none(),
        "coalesced send is still in the inbox, not yet a timeline message"
    );
    let second_receipt = service
        .receipt(&second.receipt.inbox)
        .expect("second receipt query")
        .expect("second receipt");
    assert_eq!(second_receipt.state, santi_core::receipt::State::Accepted);

    let running = service
        .snapshot(&strand.id)
        .expect("runtime snapshot")
        .expect("strand runtime");
    assert_eq!(running.turns.len(), 1);
    assert_eq!(running.turns[0].status, santi_core::turn::Status::Running);
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

    for inbox in [&first.receipt.inbox, &second.receipt.inbox] {
        let receipt = service
            .receipt(inbox)
            .expect("receipt query")
            .expect("receipt");
        assert_eq!(receipt.state, santi_core::receipt::State::Completed);
    }

    assert_eq!(runtime.turns.len(), 2);
    assert!(
        runtime
            .turns
            .iter()
            .all(|turn| turn.status == santi_core::turn::Status::Completed)
    );
    assert_eq!(count_messages(&runtime, "first request"), 1);
    assert_eq!(count_messages(&runtime, "second request"), 1);
    assert_eq!(count_messages(&runtime, "provider response 1"), 1);
    assert_eq!(count_messages(&runtime, "provider response 2"), 1);
}
