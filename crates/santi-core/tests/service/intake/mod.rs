use super::support::*;
use santi_core::message;
use santi_core::service::{self, Service};

mod downstream;
mod more;

#[tokio::test]
async fn external_ingest_turn() {
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

    let soul = service.list_souls().expect("list souls")[0].id.clone();
    let label = "github:ops:issue:PerishCode/santi#42";
    let santi_core::ingest::Outcome::Accepted { receipt } = service
        .ingest_external_event(&soul, label, "an external request arrived".to_string())
        .expect("ingest event")
    else {
        panic!("expected accepted");
    };
    let strand = receipt.strand;

    let runtime = Probe::new(&service).any_completed(&strand).await;
    assert!(
        runtime
            .turns
            .iter()
            .any(|turn| turn.trigger == santi_core::turn::Trigger::System)
    );
    assert!(
        runtime
            .messages
            .iter()
            .any(|message| message.text == "an external request arrived")
    );
    assert!(
        runtime
            .messages
            .iter()
            .any(|message| message.text == "hi from runtime")
    );

    let santi_core::ingest::Outcome::Accepted {
        receipt: receipt_again,
    } = service
        .ingest_external_event(&soul, label, "a follow-up arrived".to_string())
        .expect("ingest second event")
    else {
        panic!("expected accepted");
    };
    let strand_id_again = receipt_again.strand;
    assert_eq!(strand_id_again, strand);

    let requests = provider.requests.lock().unwrap();
    assert!(requests.iter().any(|request| {
        request.input.iter().any(|item| {
            matches!(
                item,
                Item::Message { role, content }
                    if role == "system" && content == "an external request arrived"
            )
        })
    }));
}

#[tokio::test]
async fn boot_drains_inbox() {
    let temp = tempfile::tempdir().expect("temp dir");
    let config = service::Config {
        database_path: temp.path().join("santi.sqlite").display().to_string(),
        runtime_root: temp.path().join("runtime").display().to_string(),
        execution_root: temp.path().join("execution").display().to_string(),
        bind_addr: Some("127.0.0.1:0".to_string()),
        constitution_path: None,
    };
    let provider = Arc::new(FakeProvider::default());

    let strand = {
        let service = Service::open(config.clone(), provider.clone()).expect("open service");
        service.create_strand().expect("create strand").strand.id
    };

    let store = SantiStore::open(&config.database_path).expect("open store directly");
    store
        .enqueue_inbox(
            &strand,
            message::Kind::Text,
            message::Content::text("stranded before the crash"),
        )
        .expect("enqueue inbox");
    drop(store);

    let service = Service::open(config, provider.clone()).expect("reopen service");
    service.resume_pending().expect("resume pending");

    let runtime = Probe::new(&service).any_completed(&strand).await;
    assert!(
        runtime
            .messages
            .iter()
            .any(|message| message.text == "stranded before the crash")
    );
    assert!(
        runtime
            .messages
            .iter()
            .any(|message| message.text == "hi from runtime")
    );
}
