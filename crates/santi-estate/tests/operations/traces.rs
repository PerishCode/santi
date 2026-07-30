use santi_estate::{Store, TraceDraft};
use santi_model::trace;

const FIRST: &str = "2026-07-28T00:00:00.000Z";
const LATER: &str = "2026-07-28T00:01:00.000Z";

#[tokio::test]
async fn trace_records() {
    let temp = tempfile::tempdir().expect("temp");
    let path = temp.path().join("estate.sqlite");
    let store = super::support::bootstrap(&path).await;
    let turn = trace::Tag {
        key: "turn".to_string(),
        value: "turn_test".to_string(),
    };
    let other = trace::Tag {
        key: "strand".to_string(),
        value: "strand_test".to_string(),
    };
    store
        .record_trace(TraceDraft {
            tag: "trace_two",
            boot: "boot_test",
            span: 2,
            parent: Some(1),
            name: "child",
            tags: std::slice::from_ref(&turn),
            opened: FIRST,
            closed: LATER,
        })
        .await
        .expect("child");
    store
        .record_trace(TraceDraft {
            tag: "trace_one",
            boot: "boot_test",
            span: 1,
            parent: None,
            name: "root",
            tags: &[turn, other],
            opened: FIRST,
            closed: LATER,
        })
        .await
        .expect("root");
    assert!(
        store
            .record_trace(TraceDraft {
                tag: "trace_invalid",
                boot: "boot_test",
                span: 0,
                parent: None,
                name: "invalid",
                tags: &[],
                opened: FIRST,
                closed: LATER,
            })
            .await
            .is_err()
    );

    let records = store.traces("turn", "turn_test").await.expect("records");
    assert_eq!(
        records
            .iter()
            .map(|record| record.name.as_str())
            .collect::<Vec<_>>(),
        vec!["root", "child"]
    );
    assert_eq!(
        store
            .traces("strand", "strand_test")
            .await
            .expect("strand")
            .len(),
        1
    );
    assert!(
        store
            .traces("turn", "missing")
            .await
            .expect("missing")
            .is_empty()
    );

    drop(store);
    let store = Store::open(path).await.expect("open again");
    assert_eq!(
        store
            .traces("turn", "turn_test")
            .await
            .expect("records again")
            .len(),
        2
    );
}
