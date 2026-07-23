use super::support::*;
use santi_core::service::{self, Service};
use santi_core::{message, strand};

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
            constitution_path: None,
        },
        provider.clone(),
    )
    .expect("open service");

    let mut events = service.subscribe_stream();
    let strand = service.create_strand().expect("create strand").strand;
    let response = service
        .send_strand(
            &strand.id,
            strand::Post {
                content: vec![message::Part::Text {
                    text: "say hi".to_string(),
                }],
            },
        )
        .await
        .expect("send strand");

    let turn = accepted_turn(&response).id.clone();
    let completed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await.expect("stream event").payload {
                santi_core::stream::Payload::TurnCompleted { turn, .. } => break turn,
                _ => continue,
            }
        }
    })
    .await
    .expect("turn_completed within timeout");
    assert_eq!(completed, turn);
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
            constitution_path: None,
        },
        provider.clone(),
    )
    .expect("open service");

    let mut events = service.subscribe_stream();
    let soul = service.list_souls().expect("list souls")[0].id.clone();
    let label = "github:ops:issue:PerishCode/santi#7";
    let santi_core::ingest::Outcome::Accepted { .. } = service
        .ingest_external_event(&soul, label, "external request".to_string())
        .expect("ingest event")
    else {
        panic!("expected accepted");
    };

    let (label_seen, text_seen) = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let santi_core::stream::Payload::TurnCompleted { label, text, .. } =
                events.recv().await.expect("stream event").payload
            {
                break (label, text);
            }
        }
    })
    .await
    .expect("turn_completed within timeout");
    assert_eq!(label_seen.as_deref(), Some(label));
    assert!(text_seen.is_some_and(|text| !text.is_empty()));

    let recorded = service
        .turn_events_since(0, "github:", 10)
        .expect("turn events");
    let event = recorded
        .events
        .iter()
        .find(|event| event.label == label)
        .expect("outbox turn event for the labeled strand");
    assert!(!event.text.is_empty());
}

#[tokio::test]
async fn downstream_batch_isolates_zone_and_advances_over_other_zones() {
    let temp = tempfile::tempdir().expect("temp dir");
    let service = Service::open(
        service::Config {
            database_path: temp.path().join("santi.sqlite").display().to_string(),
            runtime_root: temp.path().join("runtime").display().to_string(),
            execution_root: temp.path().join("execution").display().to_string(),
            bind_addr: Some("127.0.0.1:0".to_string()),
            constitution_path: None,
        },
        Arc::new(FakeProvider::default()),
    )
    .expect("open service");
    let soul = service.list_souls().expect("list souls")[0].id.clone();
    let mut stream = service.subscribe_stream();
    service
        .ingest_external_event(&soul, "github:issue:1", "github".to_string())
        .expect("ingest github");
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let santi_core::stream::Payload::TurnCompleted {
                label: Some(label), ..
            } = stream.recv().await.expect("stream event").payload
                && label == "github:issue:1"
            {
                break;
            }
        }
    })
    .await
    .expect("github turn completes");

    let empty = service
        .turn_events_since(0, "stim:", 10)
        .expect("empty stim batch");
    assert!(empty.events.is_empty());
    assert!(empty.cursor > 0);

    service
        .ingest_external_event(&soul, "stim:alice", "stim".to_string())
        .expect("ingest stim");
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let santi_core::stream::Payload::TurnCompleted {
                label: Some(label), ..
            } = stream.recv().await.expect("stream event").payload
                && label == "stim:alice"
            {
                break;
            }
        }
    })
    .await
    .expect("stim turn completes");

    let stim = service
        .turn_events_since(empty.cursor, "stim:", 10)
        .expect("stim batch");
    assert_eq!(stim.events.len(), 1);
    assert_eq!(stim.events[0].label, "stim:alice");
    assert!(stim.cursor > empty.cursor);
    let github = service
        .turn_events_since(0, "github:", 10)
        .expect("github batch");
    assert_eq!(github.events.len(), 1);
    assert_eq!(github.events[0].label, "github:issue:1");
    assert_eq!(github.cursor, stim.cursor);
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
            constitution_path: None,
        },
        Arc::new(FakeProvider::default()),
    )
    .expect("open service");
    let store = SantiStore::open(&database_path).expect("open store");
    store
        .open_error_incident(santi_core::error::Draft {
            key: "runtime.upgrade.failed:runtime:default".to_string(),
            descriptor: santi_core::catalog::UPGRADE_FAILED,
            scope: santi_core::Scope::new("runtime", "default"),
            source: santi_core::Source::new("santi-api", "upgrade.install"),
            message: "upgrade failed".to_string(),
            context: serde_json::json!({"attempt_id": "upgrade_test"}),
        })
        .expect("open incident");
    assert_eq!(
        santi_core::Outbox::pending(&store, 10)
            .expect("pending transitions")
            .len(),
        1
    );

    let mut events = service.subscribe_error_transitions();
    let transition = tokio::time::timeout(Duration::from_secs(1), events.recv())
        .await
        .expect("transition timeout")
        .expect("transition");
    assert_eq!(transition.held.scope.kind, "runtime");
    assert_eq!(transition.held.code, "runtime.upgrade.failed");
    assert!(
        santi_core::Outbox::pending(&store, 10)
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
            constitution_path: None,
        },
        Arc::new(FakeProvider::default()),
    )
    .expect("open service");
    let strand = service.create_strand().expect("create strand").strand;
    let store = SantiStore::open(&database_path).expect("open store");
    store
        .open_error_incident(santi_core::error::Draft {
            key: format!("test.failed:strand:{}", strand.id),
            descriptor: santi_core::catalog::INTERNAL,
            scope: santi_core::Scope::new("strand", &strand.id),
            source: santi_core::Source::new("test", "open"),
            message: "test failure".to_string(),
            context: serde_json::Value::Null,
        })
        .expect("open incident");

    let mut events = service.subscribe_error_transitions();
    let transition = tokio::time::timeout(Duration::from_secs(1), events.recv())
        .await
        .expect("transition timeout")
        .expect("transition");
    assert_eq!(transition.held.scope.id, strand.id);
}
