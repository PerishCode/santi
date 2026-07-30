use santi_estate::{
    DownstreamDraft, DrainDraft, InboxDraft, Opening, ReplayDraft, Store, StrandDraft, WebhookDraft,
};
use santi_model::{message, turn};

const SUDO: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const FIRST: &str = "2026-07-28T00:00:00.000Z";
const LATER: &str = "2026-07-28T00:01:00.000Z";

#[tokio::test]
async fn replay() {
    let temp = tempfile::tempdir().expect("temp");
    let path = temp.path().join("estate.sqlite");
    let store = Store::bootstrap(&path, SUDO).await.expect("open");
    store.seed("soul_test", FIRST).await.expect("seed");
    store.seed("soul_other", FIRST).await.expect("other soul");
    let subscription = store
        .subscribe(WebhookDraft {
            name: "github",
            adaptor: "github",
            soul: "soul_test",
            strategy: "per_thread",
            credential: "GITHUB_SECRET",
            created: FIRST,
        })
        .await
        .expect("subscribe");
    assert_eq!(subscription.soul, "soul_test");
    store
        .subscribe(WebhookDraft {
            name: "github",
            adaptor: "github",
            soul: "soul_test",
            strategy: "per_thread",
            credential: "GITHUB_SECRET",
            created: LATER,
        })
        .await
        .expect("resubscribe");
    assert!(
        store
            .subscribe(WebhookDraft {
                name: "github",
                adaptor: "github",
                soul: "soul_other",
                strategy: "per_thread",
                credential: "GITHUB_SECRET",
                created: LATER,
            })
            .await
            .is_err()
    );
    assert_eq!(store.webhooks().await.expect("webhooks").len(), 1);

    let credential = store
        .enroll(DownstreamDraft {
            tag: "worker",
            prefix: "worker/",
            digest: "credential_digest",
            created: FIRST,
        })
        .await
        .expect("enroll");
    assert_eq!(credential.prefix, "worker/");
    store
        .enroll(DownstreamDraft {
            tag: "worker",
            prefix: "worker/",
            digest: "credential_digest",
            created: LATER,
        })
        .await
        .expect("reenroll");
    assert!(
        store
            .enroll(DownstreamDraft {
                tag: "nested",
                prefix: "worker/nested/",
                digest: "nested_digest",
                created: LATER,
            })
            .await
            .is_err()
    );
    assert_eq!(store.downstreams().await.expect("downstreams").len(), 1);

    let strand = store
        .create_strand(StrandDraft {
            tag: "strand_test",
            soul: "soul_test",
            label: Some("github/thread"),
            parent: None,
            fork: None,
            created: FIRST,
        })
        .await
        .expect("strand");
    let webhook = ReplayDraft::Webhook {
        subscription: "github",
        delivery: "delivery_one",
        digest: "request_one",
    };
    let accepted = store
        .accept_replay(
            InboxDraft {
                tag: "inbox_webhook",
                strand: &strand.id,
                kind: message::Kind::SantiSystem,
                content: &message::Content::text("webhook"),
                source: None,
                created: FIRST,
            },
            webhook,
            10,
        )
        .await
        .expect("webhook");
    assert!(accepted.inserted);
    let replayed = store
        .accept_replay(
            InboxDraft {
                tag: "inbox_ignored",
                strand: &strand.id,
                kind: message::Kind::SantiSystem,
                content: &message::Content::text("ignored"),
                source: None,
                created: LATER,
            },
            webhook,
            10,
        )
        .await
        .expect("replay");
    assert!(!replayed.inserted);
    assert_eq!(replayed.receipt.inbox, "inbox_webhook");
    assert!(
        store
            .accept_replay(
                InboxDraft {
                    tag: "inbox_conflict",
                    strand: &strand.id,
                    kind: message::Kind::SantiSystem,
                    content: &message::Content::text("conflict"),
                    source: None,
                    created: LATER,
                },
                ReplayDraft::Webhook {
                    subscription: "github",
                    delivery: "delivery_one",
                    digest: "request_conflict",
                },
                10,
            )
            .await
            .is_err()
    );
    store
        .accept_replay(
            InboxDraft {
                tag: "inbox_downstream",
                strand: &strand.id,
                kind: message::Kind::Text,
                content: &message::Content::text("downstream"),
                source: None,
                created: FIRST,
            },
            ReplayDraft::Downstream {
                owner: "worker",
                request: "request_one",
                digest: "downstream_digest",
            },
            10,
        )
        .await
        .expect("downstream");

    let opened = store
        .drain_turn(DrainDraft {
            turn: "turn_test",
            strand: &strand.id,
            trigger: turn::Trigger::System,
            source: None,
            actor: "santi",
            created: LATER,
        })
        .await
        .expect("drain");
    assert!(matches!(opened, Opening::Started(_)));
    store
        .complete_turn("turn_test", 2, LATER)
        .await
        .expect("complete");
    assert!(store.inboxes(&strand.id).await.expect("empty").is_empty());
    let replayed = store
        .accept_replay(
            InboxDraft {
                tag: "inbox_after_drain",
                strand: &strand.id,
                kind: message::Kind::SantiSystem,
                content: &message::Content::text("after"),
                source: None,
                created: LATER,
            },
            webhook,
            10,
        )
        .await
        .expect("replay after drain");
    assert!(!replayed.inserted);
    assert_eq!(replayed.receipt.inbox, "inbox_webhook");

    drop(store);
    let store = Store::open(path).await.expect("open again");
    assert_eq!(store.webhooks().await.expect("webhooks").len(), 1);
    assert_eq!(store.downstreams().await.expect("downstreams").len(), 1);
}
