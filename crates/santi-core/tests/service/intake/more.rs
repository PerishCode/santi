use super::*;
use santi_core::{message, soul, strand};

#[tokio::test]
async fn pauses() {
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
        service.close();
        assert!(service.closing());
        let outcome = service
            .evented(
                "soul_default",
                "shutdown:quiesce",
                "arrived while quiescing".to_string(),
            )
            .expect("ingest during shutdown");
        match outcome {
            santi_core::ingest::Outcome::Accepted { receipt } => receipt.strand,
            other => panic!("expected accepted, got {other:?}"),
        }
    };

    let store = Store::open(&config.database).expect("open store directly");
    assert_eq!(
        store.running().expect("count"),
        0,
        "shutdown must not start a turn"
    );
    assert!(
        store.awaiting().expect("pending").contains(&strand),
        "the ingested record must still be durably queued"
    );
    drop(store);

    let service = Service::open(config, provider.clone()).expect("reopen service");
    service.resume().expect("resume pending");
    let runtime = Probe::new(&service).any_completed(&strand).await;
    assert!(
        runtime
            .messages
            .iter()
            .any(|message| message.text == "arrived while quiescing")
    );
}

#[tokio::test]
async fn targets() {
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

    let default_soul = service.souls().expect("list souls")[0].id.clone();
    let secretary = service
        .awaken(soul::Draft {
            memory: Some("# I am the secretary".to_string()),
        })
        .expect("create soul");
    assert_ne!(secretary.id, default_soul);

    let strand = service.weave().expect("create strand").strand;
    assert_eq!(strand.soul, default_soul);
    let response = service
        .send(
            &strand.id,
            strand::Post {
                content: vec![message::Part::Text {
                    text: "for whoever".to_string(),
                }],
            },
        )
        .await
        .expect("send strand");
    assert_eq!(response.strand.soul, default_soul);

    let santi_core::ingest::Outcome::Accepted {
        receipt: secretary_receipt,
    } = service
        .evented(
            &secretary.id,
            "github:issue:1",
            "hello secretary".to_string(),
        )
        .expect("ingest event")
    else {
        panic!("expected accepted");
    };
    let secretary_strand_id = secretary_receipt.strand;
    let secretary_response = service
        .send(
            &secretary_strand_id,
            strand::Post {
                content: vec![message::Part::Text {
                    text: "for the secretary".to_string(),
                }],
            },
        )
        .await
        .expect("send strand");
    assert_eq!(secretary_response.strand.soul, secretary.id);

    let error = service
        .send(
            "ss_does_not_exist",
            strand::Post {
                content: vec![message::Part::Text {
                    text: "nobody home".to_string(),
                }],
            },
        )
        .await
        .expect_err("unknown strand should error");
    assert!(error.message.contains("strand not found"), "got: {error}");
}
