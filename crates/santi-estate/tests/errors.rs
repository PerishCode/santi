use santi_error::{Draft, Kind, Scope, Source, Status, catalog};
use santi_estate::Store;

const SUDO: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const FIRST: &str = "2026-07-28T00:00:00.000Z";
const LATER: &str = "2026-07-28T00:01:00.000Z";
const RESOLVED: &str = "2026-07-28T00:02:00.000Z";
const REOPENED: &str = "2026-07-28T00:03:00.000Z";

#[tokio::test]
async fn lifecycle() {
    let temp = tempfile::tempdir().expect("temp");
    let path = temp.path().join("estate.sqlite");
    let store = Store::bootstrap(&path, SUDO).await.expect("open");
    let scope = Scope::new("strand", "strand_test");

    let first = store
        .raise(draft(&scope, "first"), FIRST)
        .await
        .expect("raise");
    let incident = first.incident.expect("incident");
    let repeated = store
        .raise(draft(&scope, "latest"), LATER)
        .await
        .expect("repeat");
    assert_eq!(repeated.incident.as_deref(), Some(incident.as_str()));
    let active = store
        .incident("runtime.internal:strand:strand_test")
        .await
        .expect("active")
        .expect("incident");
    assert_eq!(active.occurrences, 2);
    assert_eq!(active.latest.message, "latest");

    let pending = store.pending_errors(10).await.expect("pending");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].kind, Kind::Opened);
    store
        .deliver_error(&pending[0].id, LATER)
        .await
        .expect("deliver");
    store
        .deliver_error(&pending[0].id, LATER)
        .await
        .expect("redeliver");
    assert!(store.pending_errors(10).await.expect("empty").is_empty());

    assert!(
        store
            .resolve(
                "runtime.internal:strand:strand_test",
                "operator",
                serde_json::json!({"fixed": true}),
                RESOLVED,
            )
            .await
            .expect("resolve")
    );
    assert!(
        !store
            .resolve(
                "runtime.internal:strand:strand_test",
                "operator",
                serde_json::json!({}),
                RESOLVED,
            )
            .await
            .expect("resolve twice")
    );
    assert!(
        store
            .incident("runtime.internal:strand:strand_test")
            .await
            .expect("inactive")
            .is_none()
    );
    let incidents = store.incidents(&scope, 10).await.expect("history");
    assert_eq!(incidents.len(), 1);
    assert_eq!(incidents[0].status, Status::Resolved);
    assert_eq!(incidents[0].revision, 2);
    assert_eq!(
        incidents[0]
            .resolution
            .as_ref()
            .and_then(|held| held.by.as_deref()),
        Some("operator")
    );

    let reopened = store
        .raise(draft(&scope, "reopened"), REOPENED)
        .await
        .expect("reopen incident");
    assert_ne!(reopened.incident.as_deref(), Some(incident.as_str()));
    let pending = store.pending_errors(10).await.expect("pending");
    assert_eq!(
        pending.iter().map(|event| &event.kind).collect::<Vec<_>>(),
        vec![&Kind::Resolved, &Kind::Opened]
    );

    drop(store);
    let store = Store::open(path).await.expect("open again");
    let incidents = store.incidents(&scope, 10).await.expect("history again");
    assert_eq!(incidents.len(), 2);
    assert_eq!(incidents[0].status, Status::Active);
    assert_eq!(incidents[1].status, Status::Resolved);
}

fn draft(scope: &Scope, message: &str) -> Draft {
    Draft {
        key: catalog::INTERNAL.key(&scope.kind, &scope.id),
        descriptor: catalog::INTERNAL,
        scope: scope.clone(),
        source: Source::new("santi-core", "test"),
        message: message.to_string(),
        context: serde_json::json!({"message": message}),
    }
}
