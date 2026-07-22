use super::support::*;
use santi_core::service::{self, Service};

#[tokio::test]
async fn emits_turn_completed() {
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

    let mut events = service.subscribe_stream();
    let strand = service.create_strand().expect("create strand").strand;
    let response = service
        .send_strand(
            &strand.id,
            SendStrandRequest {
                content: vec![MessagePart::Text {
                    text: "say hi".to_string(),
                }],
            },
        )
        .await
        .expect("send strand");

    let turn_id = accepted_turn(&response).id.clone();
    let completed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await.expect("stream event").payload {
                santi_core::SantiStreamPayload::TurnCompleted { turn_id, .. } => break turn_id,
                _ => continue,
            }
        }
    })
    .await
    .expect("turn_completed within timeout");
    assert_eq!(completed, turn_id);
}

#[tokio::test]
async fn labeled_turn_emits_envelope_and_records_outbox() {
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

    let mut events = service.subscribe_stream();
    let soul_id = service.list_souls().expect("list souls")[0].id.clone();
    let label = "github:ops:issue:PerishCode/santi#7";
    let santi_core::IngestOutcome::Accepted { .. } = service
        .ingest_external_event(&soul_id, label, "external request".to_string())
        .expect("ingest event")
    else {
        panic!("expected accepted");
    };

    let (label_seen, text_seen) = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let santi_core::SantiStreamPayload::TurnCompleted {
                external_label,
                final_text,
                ..
            } = events.recv().await.expect("stream event").payload
            {
                break (external_label, final_text);
            }
        }
    })
    .await
    .expect("turn_completed within timeout");
    assert_eq!(label_seen.as_deref(), Some(label));
    assert!(text_seen.is_some_and(|text| !text.is_empty()));

    let recorded = service.turn_events_since(0, 10).expect("turn events");
    let event = recorded
        .iter()
        .map(|(_, event)| event)
        .find(|event| event.external_label == label)
        .expect("outbox turn event for the labeled strand");
    assert!(!event.final_text.is_empty());
}

#[tokio::test]
async fn runtime_outbox_reaches_bus() {
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
    let store = SantiStore::open(&database_path).expect("open store");
    store
        .open_error_incident(santi_core::IncidentDraft {
            incident_key: "runtime.upgrade.failed:runtime:default".to_string(),
            descriptor: santi_core::catalog::UPGRADE_FAILED,
            scope: santi_core::ErrorScope::new("runtime", "default"),
            source: santi_core::ErrorSource::new("santi-api", "upgrade.install"),
            message: "upgrade failed".to_string(),
            context: serde_json::json!({"attempt_id": "upgrade_test"}),
        })
        .expect("open incident");
    assert_eq!(
        santi_core::ErrorOutbox::pending_error_transitions(&store, 10)
            .expect("pending transitions")
            .len(),
        1
    );

    let mut events = service.subscribe_error_transitions();
    let transition = tokio::time::timeout(Duration::from_secs(1), events.recv())
        .await
        .expect("transition timeout")
        .expect("transition");
    assert_eq!(transition.incident.scope.kind, "runtime");
    assert_eq!(transition.incident.code, "runtime.upgrade.failed");
    assert!(
        santi_core::ErrorOutbox::pending_error_transitions(&store, 10)
            .expect("pending transitions")
            .is_empty()
    );
}

#[tokio::test]
async fn global_bus_sees_strands() {
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
    let strand = service.create_strand().expect("create strand").strand;
    let store = SantiStore::open(&database_path).expect("open store");
    store
        .open_error_incident(santi_core::IncidentDraft {
            incident_key: format!("test.failed:strand:{}", strand.id),
            descriptor: santi_core::catalog::INTERNAL,
            scope: santi_core::ErrorScope::new("strand", &strand.id),
            source: santi_core::ErrorSource::new("test", "open"),
            message: "test failure".to_string(),
            context: serde_json::Value::Null,
        })
        .expect("open incident");

    let mut events = service.subscribe_error_transitions();
    let transition = tokio::time::timeout(Duration::from_secs(1), events.recv())
        .await
        .expect("transition timeout")
        .expect("transition");
    assert_eq!(transition.incident.scope.id, strand.id);
}
