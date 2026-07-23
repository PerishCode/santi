use super::*;
use sha2::{Digest as _, Sha256};

fn open() -> (tempfile::TempDir, Service) {
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
    (temp, service)
}

fn register(service: &Service, id: &str, prefix: &str, token: &str) {
    service
        .create_downstream(santi_core::downstream::Draft {
            id: id.to_string(),
            prefix: prefix.to_string(),
            digest: hex::encode(Sha256::digest(token.as_bytes())),
        })
        .expect("create downstream");
}

fn receipt(admission: service::Admission) -> santi_core::ingest::Receipt {
    let service::Admission::Accepted(santi_core::ingest::Outcome::Accepted { receipt }) = admission
    else {
        panic!("expected accepted admission")
    };
    receipt
}

#[tokio::test]
async fn credential_and_zone_gate_ingest() {
    let (_temp, service) = open();
    register(&service, "stim", "stim:", "s3cret");
    let soul = service.list_souls().expect("list souls")[0].id.clone();
    let request = santi_core::ingest::Request {
        soul: soul.clone(),
        label: "stim:alice".to_string(),
        text: "hello".to_string(),
        request: "stim-message-1".to_string(),
        source: None,
    };
    assert!(matches!(
        service
            .ingest_downstream("s3cret", request.clone())
            .expect("ingest"),
        service::Admission::Accepted(_)
    ));
    assert!(matches!(
        service
            .ingest_downstream("wrong-token", request.clone())
            .expect("ingest"),
        service::Admission::Denied
    ));
    assert!(matches!(
        service
            .ingest_downstream(
                "s3cret",
                santi_core::ingest::Request {
                    label: "other:bob".to_string(),
                    request: "stim-message-2".to_string(),
                    ..request
                },
            )
            .expect("ingest"),
        service::Admission::Forbidden
    ));
}

#[tokio::test]
async fn request_key_replays_receipt_and_rejects_changed_payload() {
    let (_temp, service) = open();
    register(&service, "stim", "stim:", "s3cret");
    let soul = service.list_souls().expect("list souls")[0].id.clone();
    let request = santi_core::ingest::Request {
        soul,
        label: "stim:alice".to_string(),
        text: "hello".to_string(),
        request: "stim-message-1".to_string(),
        source: Some("message-1".to_string()),
    };
    let first = receipt(
        service
            .ingest_downstream("s3cret", request.clone())
            .expect("first ingest"),
    );
    let repeated = receipt(
        service
            .ingest_downstream("s3cret", request.clone())
            .expect("repeated ingest"),
    );
    assert_eq!(repeated.strand, first.strand);
    assert_eq!(repeated.inbox, first.inbox);
    let strand_count = service.list_strands().expect("list strands").len();
    let error = match service.ingest_downstream(
        "s3cret",
        santi_core::ingest::Request {
            label: "stim:bob".to_string(),
            text: "changed".to_string(),
            ..request
        },
    ) {
        Err(error) => error,
        Ok(_) => panic!("changed replay must conflict"),
    };
    assert!(error.contains("conflicts with an accepted payload"));
    assert_eq!(
        service.list_strands().expect("list strands").len(),
        strand_count
    );
}

#[test]
fn registration_is_unique_idempotent_and_secret_free() {
    let (_temp, service) = open();
    let digest = hex::encode(Sha256::digest(b"s3cret"));
    let request = santi_core::downstream::Draft {
        id: "stim".to_string(),
        prefix: "stim:".to_string(),
        digest: digest.clone(),
    };
    let created = service
        .create_downstream(request.clone())
        .expect("create downstream");
    let repeated = service
        .create_downstream(request)
        .expect("repeat registration");
    assert_eq!(repeated.id, created.id);
    let exposed = serde_json::to_value(&created).expect("serialize downstream");
    assert!(exposed.get("digest").is_none());
    let decoded: santi_core::downstream::Credential =
        serde_json::from_value(exposed).expect("deserialize public downstream");
    assert!(decoded.digest.is_empty());
    let overlap = service
        .create_downstream(santi_core::downstream::Draft {
            id: "nested".to_string(),
            prefix: "stim:nested:".to_string(),
            digest: hex::encode(Sha256::digest(b"nested")),
        })
        .expect_err("overlap must fail");
    assert!(overlap.contains("overlaps"));
    let reused = service
        .create_downstream(santi_core::downstream::Draft {
            id: "github".to_string(),
            prefix: "github:".to_string(),
            digest,
        })
        .expect_err("credential reuse must fail");
    assert!(reused.contains("already registered"));
}
