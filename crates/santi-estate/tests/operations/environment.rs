use santi_estate::{EnvironDraft, Store, StrandDraft};
use santi_model::environ;

#[tokio::test]
async fn ownership() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = Store::open(temp.path().join("santi.sqlite"))
        .await
        .expect("open store");
    store
        .seed("soul_default", "2026-07-29T00:00:00.000Z")
        .await
        .expect("seed soul");
    store
        .create_strand(StrandDraft {
            tag: "ss_env",
            soul: "soul_default",
            label: None,
            parent: None,
            fork: None,
            created: "2026-07-29T00:00:00.000Z",
        })
        .await
        .expect("create strand");

    let soul = store
        .set_environ(EnvironDraft {
            scope: environ::Scope::Soul,
            owner: "soul_default",
            name: "SHARED",
            value: "soul",
            occurred: "2026-07-29T00:00:01.000Z",
        })
        .await
        .expect("set soul environment");
    assert_eq!(soul.value, "soul");
    assert_eq!(soul.created, soul.updated);

    let strand = store
        .set_environ(EnvironDraft {
            scope: environ::Scope::Strand,
            owner: "ss_env",
            name: "SHARED",
            value: "strand",
            occurred: "2026-07-29T00:00:02.000Z",
        })
        .await
        .expect("set strand environment");
    assert_eq!(strand.value, "strand");

    let updated = store
        .set_environ(EnvironDraft {
            scope: environ::Scope::Soul,
            owner: "soul_default",
            name: "SHARED",
            value: "changed",
            occurred: "2026-07-29T00:00:03.000Z",
        })
        .await
        .expect("update soul environment");
    assert_eq!(updated.created, "2026-07-29T00:00:01.000Z");
    assert_eq!(updated.updated, "2026-07-29T00:00:03.000Z");
    assert_eq!(
        store
            .environs(environ::Scope::Soul, "soul_default")
            .await
            .expect("list soul environment"),
        vec![updated]
    );

    assert!(
        store
            .end_environ(environ::Scope::Soul, "soul_default", "SHARED")
            .await
            .expect("end environment")
    );
    assert!(
        !store
            .end_environ(environ::Scope::Soul, "soul_default", "SHARED")
            .await
            .expect("repeat end")
    );
    assert!(
        store
            .environs(environ::Scope::Strand, "missing")
            .await
            .expect_err("missing owner must refuse")
            .contains("strand not found")
    );
}
