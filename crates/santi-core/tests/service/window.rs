use super::support::*;
use santi_core::service::window::{Outcome, window_participant};
use santi_core::{WindowAuthor, WindowSendRequest, service};

const UID: &str = "operator-uid-fixture";

fn open_service(temp: &tempfile::TempDir) -> service::Service {
    service::Service::open(
        service::Config {
            database_path: temp.path().join("santi.sqlite").display().to_string(),
            runtime_root: temp.path().join("runtime").display().to_string(),
            execution_root: temp.path().join("execution").display().to_string(),
            bind_addr: Some("127.0.0.1:0".to_string()),
        },
        Arc::new(FakeProvider::default()),
    )
    .expect("open service")
}

fn send(text: &str, key: &str) -> WindowSendRequest {
    WindowSendRequest {
        content: text.to_string(),
        client_message_id: key.to_string(),
    }
}

fn participant_strand(service: &service::Service, participant: &str) -> String {
    let label = format!("im:{participant}");
    service
        .list_strands()
        .expect("list strands")
        .into_iter()
        .find(|strand| strand.external_label.as_deref() == Some(label.as_str()))
        .expect("participant strand")
        .id
}

async fn completed_count(service: &service::Service, strand_id: &str, wanted: usize) {
    for _ in 0..100 {
        let runtime = service
            .runtime_snapshot(strand_id)
            .expect("runtime")
            .expect("strand");
        let done = runtime
            .turns
            .iter()
            .filter(|turn| turn.status == santi_core::TurnStatus::Completed)
            .count();
        if done >= wanted {
            return;
        }
        sleep(Duration::from_millis(20)).await;
    }
    panic!("turn count never reached {wanted}");
}

#[test]
fn participant_is_stable_and_opaque() {
    let first = window_participant(UID);
    let second = window_participant(UID);
    assert_eq!(first, second);
    assert!(first.starts_with("window:"));
    assert_eq!(first.len(), "window:".len() + 32);
    assert_ne!(first, window_participant("someone-else"));
}

#[tokio::test]
async fn accepts_and_materializes() {
    let temp = tempfile::tempdir().expect("temp dir");
    let service = open_service(&temp);
    let Outcome::Accepted(accepted) = service
        .window_send(UID, send("hello through the wall", "key-1"))
        .expect("send")
    else {
        panic!("send rejected");
    };
    assert_eq!(accepted.status, "accepted");
    assert_eq!(accepted.cursor, None, "cursor is unknown at acceptance");
    assert!(accepted.receipt_id.starts_with("inbox"));

    let participant = window_participant(UID);
    let strand_id = participant_strand(&service, &participant);
    Probe::new(&service).any_completed(&strand_id).await;

    let transcript = service.window_transcript(UID, 0, 200).expect("transcript");
    assert!(!transcript.empty);
    let human = transcript
        .entries
        .iter()
        .find(|entry| entry.author == WindowAuthor::Human)
        .expect("human entry");
    assert_eq!(
        human.message_id, accepted.message_id,
        "drain reuses the reserved id"
    );
    assert_eq!(human.text, "hello through the wall");
    assert_eq!(
        human.at, accepted.received_at,
        "human entries show acceptance time"
    );
    assert!(
        transcript
            .entries
            .iter()
            .any(|entry| entry.author == WindowAuthor::Assistant),
        "assistant reply projected"
    );

    let Outcome::Accepted(replayed) = service
        .window_send(UID, send("hello through the wall", "key-1"))
        .expect("replay")
    else {
        panic!("replay rejected");
    };
    assert_eq!(replayed.message_id, accepted.message_id);
    assert_eq!(replayed.receipt_id, accepted.receipt_id);
    assert_eq!(replayed.received_at, accepted.received_at);
    assert!(replayed.cursor.is_some(), "cursor backfilled after drain");
    assert_eq!(replayed.cursor, Some(human.seq));
}

#[tokio::test]
async fn conflict_and_validation_ladder() {
    let temp = tempfile::tempdir().expect("temp dir");
    let service = open_service(&temp);
    let Outcome::Accepted(_) = service
        .window_send(UID, send("original", "key-c"))
        .expect("send")
    else {
        panic!("send rejected");
    };
    let Outcome::Rejected(conflict) = service
        .window_send(UID, send("different", "key-c"))
        .expect("conflict call")
    else {
        panic!("conflict accepted");
    };
    assert_eq!(conflict.code, "window.message.conflict");

    let Outcome::Rejected(blank) = service
        .window_send(UID, send("   ", "key-b"))
        .expect("blank call")
    else {
        panic!("blank accepted");
    };
    assert_eq!(blank.code, "window.content.invalid");

    let Outcome::Rejected(oversize) = service
        .window_send(UID, send(&"x".repeat(16 * 1024 + 1), "key-o"))
        .expect("oversize call")
    else {
        panic!("oversize accepted");
    };
    assert_eq!(oversize.code, "window.content.oversize");

    let Outcome::Rejected(badkey) = service
        .window_send(UID, send("fine", &"k".repeat(129)))
        .expect("badkey call")
    else {
        panic!("bad key accepted");
    };
    assert_eq!(badkey.code, "window.content.invalid");

    let Outcome::Rejected(noid) = service
        .window_send("  ", send("fine", "key-n"))
        .expect("noid call")
    else {
        panic!("blank uid accepted");
    };
    assert_eq!(noid.code, "window.identity.missing");
}

#[tokio::test]
async fn rate_limits_after_burst_but_replays_freely() {
    let temp = tempfile::tempdir().expect("temp dir");
    let service = open_service(&temp);
    for index in 0..5 {
        let outcome = service
            .window_send(UID, send("burst message", &format!("burst-{index}")))
            .expect("burst send");
        assert!(
            matches!(outcome, Outcome::Accepted(_)),
            "burst {index} accepted"
        );
    }
    let Outcome::Rejected(limited) = service
        .window_send(UID, send("one too many", "burst-5"))
        .expect("limited call")
    else {
        panic!("sixth send accepted");
    };
    assert_eq!(limited.code, "window.rate.limited");
    assert!(limited.context["retry_after_seconds"].as_u64().unwrap() >= 1);

    let outcome = service
        .window_send(UID, send("burst message", "burst-0"))
        .expect("replay while limited");
    assert!(
        matches!(outcome, Outcome::Accepted(_)),
        "replays never consume rate tokens"
    );
}

#[tokio::test]
async fn transcript_excludes_runtime_kinds_and_paginates() {
    let temp = tempfile::tempdir().expect("temp dir");
    let service = open_service(&temp);

    let before = service.window_transcript(UID, 0, 200).expect("transcript");
    assert!(before.empty, "unknown participant has an empty transcript");
    assert_eq!(before.next_since, 0);

    for index in 0..3 {
        let outcome = service
            .window_send(
                UID,
                send(&format!("message {index}"), &format!("page-{index}")),
            )
            .expect("send");
        assert!(matches!(outcome, Outcome::Accepted(_)));
        let participant = window_participant(UID);
        let strand_id = participant_strand(&service, &participant);
        completed_count(&service, &strand_id, index + 1).await;
    }

    let participant = window_participant(UID);
    let strand_id = participant_strand(&service, &participant);
    service
        .ingest_external_source(
            santi_core::DEFAULT_SOUL_ID,
            &format!("im:{participant}"),
            "runtime notice fixture".to_string(),
            None,
        )
        .expect("santi system ingest");
    completed_count(&service, &strand_id, 4).await;

    let all = service.window_transcript(UID, 0, 200).expect("transcript");
    assert!(!all.empty);
    assert!(
        all.entries
            .iter()
            .all(|entry| entry.text != "runtime notice fixture"),
        "santi_system messages stay out of the chat projection"
    );

    let page = service.window_transcript(UID, 0, 2).expect("page");
    assert_eq!(page.entries.len(), 2);
    assert!(page.has_more);
    assert!(!page.empty);
    let rest = service
        .window_transcript(UID, page.next_since, 200)
        .expect("rest");
    assert!(!rest.entries.is_empty());
    assert!(
        rest.entries.first().unwrap().seq > page.next_since,
        "since is exclusive"
    );

    let quiet = service
        .window_transcript(UID, all.next_since, 200)
        .expect("quiet");
    assert!(quiet.entries.is_empty());
    assert!(!quiet.empty, "no-new is not the same as no-history");
    assert_eq!(quiet.next_since, all.next_since);
}
