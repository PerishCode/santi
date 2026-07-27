use super::support::*;
use santi_core::message;
use santi_core::service::{self, Service};

mod downstream;
mod more;

#[tokio::test]
async fn ingests() {
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

    let soul = service.souls().expect("list souls")[0].id.clone();
    let label = "github:ops:issue:PerishCode/santi#42";
    let santi_core::ingest::Outcome::Accepted { receipt } = service
        .evented(&soul, label, "an external request arrived".to_string())
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
        .evented(&soul, label, "a follow-up arrived".to_string())
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
async fn drains() {
    let temp = tempfile::tempdir().expect("temp dir");
    let config = service::Config {
        database: temp.path().join("santi.sqlite").display().to_string(),
        runtime: temp.path().join("runtime").display().to_string(),
        execution: temp.path().join("execution").display().to_string(),
        bind: Some("127.0.0.1:0".to_string()),
        constitution: None,
    };
    let provider = Arc::new(FakeProvider::default());

    let strand = {
        let service = Service::open(config.clone(), provider.clone()).expect("open service");
        service.weave().expect("create strand").strand.id
    };

    let store = Store::open(&config.database).expect("open store directly");
    store
        .receive(
            &strand,
            message::Kind::Text,
            message::Content::text("stranded before the crash"),
            None,
        )
        .expect("enqueue inbox");
    drop(store);

    let service = Service::open(config, provider.clone()).expect("reopen service");
    service.resume().expect("resume pending");

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
