use super::super::support::*;
use super::service_with_budget;
use santi_core::{effect, message, strand};

#[tokio::test]
async fn redrives() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db = temp.path().join("santi.sqlite");
    let provider = Arc::new(LargeToolCallProvider {
        requests: Arc::new(Mutex::new(Vec::new())),
        bytes: 100_000,
    });
    let service = service_with_budget(&temp, provider.clone());
    let strand = service.weave().expect("create strand").strand;
    let response = service
        .send(
            &strand.id,
            strand::Post {
                content: vec![message::Part::Text {
                    text: "one compact-recoverable obligation".to_string(),
                }],
            },
        )
        .await
        .expect("send strand");
    let runtime = Probe::new(&service)
        .failed_turn(&strand.id, &accepted_turn(&response).id)
        .await;
    let receipt = service
        .receipt(&response.receipt.inbox)
        .expect("receipt query")
        .expect("receipt");
    assert_eq!(receipt.state, santi_core::receipt::State::Failed);

    let first = runtime
        .messages
        .iter()
        .find(|message| message.text == "one compact-recoverable obligation")
        .expect("user message")
        .message
        .id
        .clone();
    let store = Store::open(&db).expect("open store directly");
    let boundary = store
        .pen(Draft {
            strand: &strand.id,
            actor: message::Role::Soul,
            id: store.genesis(),
            content: message::Content::text("manual compact boundary"),
            state: message::State::Fixed,
            intake: message::Intake::Record,
        })
        .expect("append manual boundary")
        .message;

    let compact = service
        .exec(
            &strand.id,
            santi_core::compact::Exec {
                first: Some(first),
                last: Some(boundary.message.id),
                from: None,
                to: None,
                summary: "Failed tool exchange compacted for explicit recovery.".to_string(),
                capsule: None,
                dry: false,
            },
        )
        .expect("compact should resolve and redrive");
    assert!(compact.active_incident_resolved);

    let runtime = Probe::new(&service).any_completed(&strand.id).await;
    assert_eq!(provider.requests.lock().unwrap().len(), 2);
    assert_eq!(runtime.effects.len(), 1, "effect must not replay");
    assert_eq!(
        runtime.effects[0].state,
        effect::State::Settled(effect::Outcome::Applied)
    );
    assert_eq!(runtime.errors[0].status, santi_core::Status::Resolved);
    let receipt = service
        .receipt(&response.receipt.inbox)
        .expect("receipt query")
        .expect("receipt");
    assert_eq!(receipt.state, santi_core::receipt::State::Completed);
    assert_eq!(
        receipt
            .transitions
            .iter()
            .map(|transition| transition.state.clone())
            .collect::<Vec<_>>(),
        vec![
            santi_core::receipt::State::Accepted,
            santi_core::receipt::State::Driving,
            santi_core::receipt::State::Failed,
            santi_core::receipt::State::Driving,
            santi_core::receipt::State::Completed,
        ]
    );
}
