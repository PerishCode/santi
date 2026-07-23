use santi_core::{Store, catalog};

#[test]
fn generic_ports_are_transactional() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = Store::open(temp.path().join("santi.sqlite")).expect("open store");
    let scope = santi_core::Scope::new("runtime", "default");
    let key = "runtime.upgrade.failed:runtime:default";
    let draft = || santi_core::error::Draft {
        key: key.to_string(),
        descriptor: catalog::UPGRADE_FAILED,
        scope: scope.clone(),
        source: santi_core::Source::new("santi-api", "upgrade.install"),
        message: "upgrade install failed".to_string(),
        context: serde_json::json!({"attempt_id": "upgrade_1"}),
    };

    let opened = store.open_error_incident(draft()).expect("open incident");
    let repeated = store.open_error_incident(draft()).expect("repeat incident");
    assert_eq!(opened.incident, repeated.incident);

    let incidents = store.incidents(&scope, 10).expect("list incidents");
    assert_eq!(incidents.len(), 1);
    assert_eq!(incidents[0].occurrences, 2);
    assert_eq!(incidents[0].revision, 1);

    assert!(
        store
            .resolve_error_incident(
                key,
                "upgrade.succeeded",
                serde_json::json!({"attempt_id": "upgrade_2"}),
            )
            .expect("resolve incident")
    );
    let incidents = store.incidents(&scope, 10).expect("list resolved");
    assert_eq!(incidents[0].status, santi_core::Status::Resolved);
    assert_eq!(incidents[0].revision, 2);
}
