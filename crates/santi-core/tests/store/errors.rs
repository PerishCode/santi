use santi_core::{ErrorScope, ErrorSource, IncidentDraft, IncidentStatus, SantiStore, catalog};

#[test]
fn generic_ports_are_transactional() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = SantiStore::open(temp.path().join("santi.sqlite")).expect("open store");
    let scope = ErrorScope::new("runtime", "default");
    let key = "runtime.upgrade.failed:runtime:default";
    let draft = || IncidentDraft {
        incident_key: key.to_string(),
        descriptor: catalog::UPGRADE_FAILED,
        scope: scope.clone(),
        source: ErrorSource::new("santi-api", "upgrade.install"),
        message: "upgrade install failed".to_string(),
        context: serde_json::json!({"attempt_id": "upgrade_1"}),
    };

    let opened = store.open_error_incident(draft()).expect("open incident");
    let repeated = store.open_error_incident(draft()).expect("repeat incident");
    assert_eq!(opened.incident_id, repeated.incident_id);

    let incidents = store.error_incidents(&scope, 10).expect("list incidents");
    assert_eq!(incidents.len(), 1);
    assert_eq!(incidents[0].occurrence_count, 2);
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
    let incidents = store.error_incidents(&scope, 10).expect("list resolved");
    assert_eq!(incidents[0].status, IncidentStatus::Resolved);
    assert_eq!(incidents[0].revision, 2);
}
