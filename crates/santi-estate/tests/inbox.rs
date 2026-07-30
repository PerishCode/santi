use santi_estate::{InboxDraft, NoticeDraft, Store, StrandDraft, TurnDraft};
use santi_model::{ingest, message, receipt, turn};

const SUDO: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const FIRST: &str = "2026-07-28T00:00:00.000Z";
const LATER: &str = "2026-07-28T00:01:00.000Z";

#[tokio::test]
async fn admission() {
    let temp = tempfile::tempdir().expect("temp");
    let path = temp.path().join("estate.sqlite");
    let store = Store::bootstrap(&path, SUDO).await.expect("open");
    store.seed("soul_test", FIRST).await.expect("seed");
    let strand = store
        .create_strand(StrandDraft {
            tag: "strand_test",
            soul: "soul_test",
            label: None,
            parent: None,
            fork: None,
            created: FIRST,
        })
        .await
        .expect("strand");

    let text = message::Content::text("hello");
    let inbox = store
        .accept_inbox(
            InboxDraft {
                tag: "inbox_text",
                strand: &strand.id,
                kind: message::Kind::Text,
                content: &text,
                source: None,
                created: FIRST,
            },
            2,
        )
        .await
        .expect("accept");
    assert_eq!(inbox.content.rendered(), "hello");
    let status = store
        .receipt(&inbox.id)
        .await
        .expect("receipt")
        .expect("status");
    assert_eq!(status.state, receipt::State::Accepted);
    assert_eq!(status.transitions.len(), 1);

    let first_source = ingest::Source::new("job")
        .with_ref("job_test")
        .with_metadata(serde_json::json!({"phase": 1}));
    let causes = vec!["slow".to_string()];
    let offered = store
        .offer_notice(
            NoticeDraft {
                tag: "inbox_notice",
                strand: &strand.id,
                key: "attention",
                revision: 1,
                digest: "digest_1",
                content: &message::Content::text("notice one"),
                source: &first_source,
                causes: &causes,
                created: FIRST,
            },
            2,
        )
        .await
        .expect("offer");
    assert!(offered.inserted);
    assert_eq!(offered.inbox.as_deref(), Some("inbox_notice"));
    let repeated = store
        .offer_notice(
            NoticeDraft {
                tag: "inbox_ignored",
                strand: &strand.id,
                key: "attention",
                revision: 1,
                digest: "digest_1",
                content: &message::Content::text("ignored"),
                source: &first_source,
                causes: &[],
                created: LATER,
            },
            2,
        )
        .await
        .expect("repeat");
    assert_eq!(repeated.inbox.as_deref(), Some("inbox_notice"));
    assert!(!repeated.inserted);
    assert!(
        store
            .offer_notice(
                NoticeDraft {
                    tag: "inbox_conflict",
                    strand: &strand.id,
                    key: "attention",
                    revision: 1,
                    digest: "digest_conflict",
                    content: &message::Content::text("conflict"),
                    source: &first_source,
                    causes: &[],
                    created: LATER,
                },
                2,
            )
            .await
            .is_err()
    );

    let later_source = ingest::Source::new("job");
    let later_causes = vec!["large".to_string(), "slow".to_string()];
    let advanced = store
        .offer_notice(
            NoticeDraft {
                tag: "inbox_still_ignored",
                strand: &strand.id,
                key: "attention",
                revision: 2,
                digest: "digest_2",
                content: &message::Content::text("notice two"),
                source: &later_source,
                causes: &later_causes,
                created: LATER,
            },
            2,
        )
        .await
        .expect("advance");
    assert_eq!(advanced.inbox.as_deref(), Some("inbox_notice"));
    let notice = store
        .inbox("inbox_notice")
        .await
        .expect("notice")
        .expect("pending");
    assert_eq!(notice.content.rendered(), "notice two");
    assert_eq!(notice.coalesce_revision, Some(2));
    assert_eq!(notice.coalesce_causes, vec!["large", "slow"]);
    assert_eq!(notice.source.expect("source").source, None);
    assert!(
        store
            .accept_inbox(
                InboxDraft {
                    tag: "inbox_overflow",
                    strand: &strand.id,
                    kind: message::Kind::Text,
                    content: &text,
                    source: None,
                    created: LATER,
                },
                2,
            )
            .await
            .is_err()
    );

    let turn = store
        .create_turn(TurnDraft {
            tag: "turn_test",
            strand: &strand.id,
            trigger: turn::Trigger::System,
            source: None,
            from: 0,
            created: LATER,
        })
        .await
        .expect("turn");
    let receipt = store
        .advance_receipt(santi_estate::ReceiptDraft {
            inbox: "inbox_notice",
            state: receipt::State::Driving,
            turn: Some(&turn.id),
            incident: None,
            rebuilt: None,
            occurred: LATER,
        })
        .await
        .expect("drive");
    assert_eq!(receipt.state, receipt::State::Driving);
    assert_eq!(receipt.transitions.len(), 2);
    assert_eq!(receipt.transitions[1].turn.as_deref(), Some("turn_test"));

    drop(store);
    let store = Store::open(path).await.expect("open again");
    assert_eq!(store.inboxes(&strand.id).await.expect("pending").len(), 2);
    assert_eq!(
        store
            .receipt("inbox_notice")
            .await
            .expect("receipt again")
            .expect("status")
            .state,
        receipt::State::Driving
    );
}
