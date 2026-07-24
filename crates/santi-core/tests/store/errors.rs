use santi_core::{Store, catalog};

#[test]
fn generic_ports_are_transactional() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = Store::open(temp.path().join("santi.sqlite")).expect("open store");
    let scope = santi_core::Scope::new("runtime", "default");
    let key = "runtime.internal:runtime:default";
    let draft = || santi_core::error::Draft {
        key: key.to_string(),
        descriptor: catalog::INTERNAL,
        scope: scope.clone(),
        source: santi_core::Source::new("santi-core", "runtime.boot"),
        message: "runtime boot failed".to_string(),
        context: serde_json::json!({"phase": "one"}),
    };

    let opened = store.raise(draft()).expect("open incident");
    let repeated = store.raise(draft()).expect("repeat incident");
    assert_eq!(opened.incident, repeated.incident);

    let incidents = store.incidents(&scope, 10).expect("list incidents");
    assert_eq!(incidents.len(), 1);
    assert_eq!(incidents[0].occurrences, 2);
    assert_eq!(incidents[0].revision, 1);

    assert!(
        store
            .resolve(
                key,
                "runtime.recovered",
                serde_json::json!({"phase": "two"}),
            )
            .expect("resolve incident")
    );
    let incidents = store.incidents(&scope, 10).expect("list resolved");
    assert_eq!(incidents[0].status, santi_core::Status::Resolved);
    assert_eq!(incidents[0].revision, 2);
}
