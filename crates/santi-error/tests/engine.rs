use santi_error::{Draft, Kind, Scope, Source, Status, catalog, engine};
use serde_json::json;

fn draft(operation: &str) -> Draft {
    Draft {
        key: "context.budget.exceeded:strand:ss_1".to_string(),
        descriptor: catalog::CONTEXT_BUDGET_EXCEEDED,
        scope: Scope::new("strand", "ss_1"),
        source: Source::new("santi-core", operation),
        message: "over budget".to_string(),
        context: json!({"phase": operation}),
    }
}

#[test]
fn repeat_is_idempotent() {
    let opened = engine().open(None, draft("ingest_admission"), "t1");
    let repeated = engine().open(Some(&opened.incident), draft("active_guard"), "t2");

    assert_eq!(repeated.incident.id, opened.incident.id);
    assert_eq!(repeated.incident.revision, 1);
    assert_eq!(repeated.incident.occurrences, 2);
    assert!(repeated.transition.is_none());
    assert_eq!(repeated.incident.first.source.operation, "ingest_admission");
    assert_eq!(repeated.incident.latest.source.operation, "active_guard");
}

#[test]
fn resolve_advances_revision() {
    let opened = engine().open(None, draft("provider_preflight"), "t1");
    let resolved = engine().resolve(
        &opened.incident,
        "compact_exec",
        json!({"total_bytes": 42}),
        "t2",
    );

    assert_eq!(resolved.incident.status, Status::Resolved);
    assert_eq!(resolved.incident.revision, 2);
    assert_eq!(resolved.transition.unwrap().kind, Kind::Resolved);
}
