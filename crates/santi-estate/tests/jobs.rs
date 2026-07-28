use santi_estate::{
    AttentionDraft, CallDraft, CapabilityDraft, EffectDraft, JobDraft, NoticeDraft, Prepared,
    Store, StrandDraft, TransitionDraft, TurnDraft,
};
use santi_model::{ingest, job, message, turn};

const FIRST: &str = "2026-07-28T00:00:00.000Z";
const LATER: &str = "2026-07-28T00:01:00.000Z";
const FINISHED: &str = "2026-07-28T00:02:00.000Z";
const ACKED: &str = "2026-07-28T00:03:00.000Z";

#[tokio::test]
async fn lifecycle() {
    let temp = tempfile::tempdir().expect("temp");
    let path = temp.path().join("estate.sqlite");
    let store = Store::open(&path).await.expect("open");
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
    let turn = store
        .create_turn(TurnDraft {
            tag: "turn_test",
            strand: &strand.id,
            trigger: turn::Trigger::System,
            source: None,
            from: 0,
            created: FIRST,
        })
        .await
        .expect("turn");
    let call = store
        .create_call(CallDraft {
            tag: "call_test",
            turn: &turn.id,
            tool: "shell",
            arguments: &serde_json::json!({"command": "true"}),
            created: FIRST,
        })
        .await
        .expect("call");
    let effect = store
        .prepare_effect(EffectDraft {
            tag: "effect_test",
            turn: &turn.id,
            call: Some(&call.id),
            kind: "shell",
            metadata: None,
            created: FIRST,
        })
        .await
        .expect("effect");
    store
        .create_capability(CapabilityDraft {
            digest: "capability_digest",
            expires: 1_000,
            soul: "soul_test",
            strand: &strand.id,
            turn: &turn.id,
            call: &call.id,
            effect: &effect.id,
            created: FIRST,
        })
        .await
        .expect("capability");

    let draft = JobDraft {
        tag: "job_test",
        description: "test job",
        command: "true",
        cwd: Some("/tmp"),
        timeout_seconds: 30,
        output_limit_bytes: 1024,
        remind_every_seconds: Some(5),
        request_sha256: "request_digest",
        generation: "stamp_test",
        supervisor_ref: "santi-stamp-test.service",
        created: FIRST,
    };
    let Prepared::New(created) = store
        .prepare_job("capability_digest", draft, 500)
        .await
        .expect("prepare")
    else {
        panic!("first prepare must create");
    };
    assert_eq!(created.job.state, job::State::Submitting);
    assert_eq!(created.job.origin.soul, "soul_test");
    let Prepared::Existing(replayed) = store
        .prepare_job("capability_digest", draft, 500)
        .await
        .expect("replay")
    else {
        panic!("replay must reuse");
    };
    assert_eq!(replayed.job.id, "job_test");
    assert!(
        store
            .prepare_job(
                "capability_digest",
                JobDraft {
                    request_sha256: "conflict",
                    ..draft
                },
                500,
            )
            .await
            .is_err()
    );

    let accepted = store.accept_job("job_test", LATER).await.expect("accept");
    assert_eq!(accepted.job.state, job::State::Accepted);
    assert_eq!(accepted.job.accepted.as_deref(), Some(LATER));
    let accepted_again = store
        .accept_job("job_test", FINISHED)
        .await
        .expect("accept again");
    assert_eq!(accepted_again.job.accepted.as_deref(), Some(LATER));

    let running = store
        .transition_job(
            "job_test",
            TransitionDraft {
                state: job::State::Running,
                reason: None,
                exit_code: None,
                occurred: LATER,
                started_millis: Some(700),
                next_reminder: Some(FINISHED),
            },
        )
        .await
        .expect("running");
    assert_eq!(running.job.started.as_deref(), Some(LATER));
    assert_eq!(running.started_millis, Some(700));
    assert_eq!(running.job.next.as_deref(), Some(FINISHED));
    assert_eq!(store.active_jobs().await.expect("active").len(), 1);

    let source = ingest::Source::new("job").with_ref("job_test");
    let causes = vec!["runtime".to_string()];
    let notice = NoticeDraft {
        tag: "inbox_attention",
        strand: &strand.id,
        key: "job:job_test",
        revision: 1,
        digest: "attention_digest",
        content: &message::Content::text("job attention"),
        source: &source,
        causes: &causes,
        created: LATER,
    };
    let offered = store
        .attend_job(
            AttentionDraft {
                job: "job_test",
                base: 0,
                at: LATER,
                runtime: true,
                output: false,
                reminded: true,
                tick: 1,
                next: Some(FINISHED),
            },
            notice,
            10,
        )
        .await
        .expect("attention");
    assert!(offered.inserted);
    let attended = store.active_jobs().await.expect("attended").remove(0);
    assert_eq!(attended.attention_revision, 1);
    assert!(attended.runtime_warned);
    assert_eq!(attended.reminder_tick, 1);
    assert_eq!(attended.job.last.as_deref(), Some(LATER));
    assert_eq!(attended.job.next.as_deref(), Some(FINISHED));
    assert!(
        !store
            .attend_job(
                AttentionDraft {
                    job: "job_test",
                    base: 0,
                    at: LATER,
                    runtime: true,
                    output: false,
                    reminded: true,
                    tick: 1,
                    next: Some(FINISHED),
                },
                notice,
                10,
            )
            .await
            .expect("attention replay")
            .inserted
    );
    assert!(
        store
            .attend_job(
                AttentionDraft {
                    job: "job_test",
                    base: 2,
                    at: LATER,
                    runtime: false,
                    output: true,
                    reminded: false,
                    tick: 1,
                    next: None,
                },
                NoticeDraft {
                    revision: 3,
                    ..notice
                },
                10,
            )
            .await
            .is_err()
    );

    let succeeded = store
        .transition_job(
            "job_test",
            TransitionDraft {
                state: job::State::Succeeded,
                reason: Some("completed"),
                exit_code: Some(0),
                occurred: FINISHED,
                started_millis: None,
                next_reminder: None,
            },
        )
        .await
        .expect("succeed");
    assert_eq!(succeeded.job.finished.as_deref(), Some(FINISHED));
    assert_eq!(succeeded.job.next, None);
    assert!(store.active_jobs().await.expect("inactive").is_empty());
    let other = store.job("other_soul", "job_test").await.expect("other");
    assert!(other.is_none());
    assert_eq!(store.jobs("soul_test").await.expect("jobs").len(), 1);

    let acknowledged = store.acknowledge_job("job_test", ACKED).await.expect("ack");
    assert_eq!(acknowledged.job.acknowledged.as_deref(), Some(ACKED));
    store
        .acknowledge_job("job_test", "2026-07-28T00:04:00.000Z")
        .await
        .expect("ack twice");
    assert_eq!(
        store.expired_jobs(ACKED, 10).await.expect("expired"),
        vec![santi_estate::ExpiredJob {
            id: "job_test".to_string(),
            key: "stamp_test".to_string(),
        }]
    );
    assert!(store.retained_job("stamp_test").await.expect("retained"));
    let early = store
        .purge_job("job_test", FINISHED)
        .await
        .expect("too early");
    assert!(!early);
    assert!(store.purge_job("job_test", ACKED).await.expect("purge"));
    assert!(!store.retained_job("stamp_test").await.expect("gone"));
    assert!(
        store
            .prepare_job("capability_digest", draft, 500)
            .await
            .is_err()
    );

    store
        .create_capability(CapabilityDraft {
            digest: "expired_capability",
            expires: 10,
            soul: "soul_test",
            strand: &strand.id,
            turn: &turn.id,
            call: &call.id,
            effect: &effect.id,
            created: FIRST,
        })
        .await
        .expect("expiring capability");
    assert_eq!(
        store
            .expire_capabilities(11)
            .await
            .expect("expire capabilities"),
        1
    );

    drop(store);
    Store::open(path).await.expect("open again");
}
