use santi_error::{
    ErrorScope, ErrorSource, ErrorTransitionKind, IncidentDraft, IncidentStatus, catalog, engine,
};
use serde_json::json;

fn draft(operation: &str) -> IncidentDraft {
    IncidentDraft {
        incident_key: "context.budget.exceeded:strand:ss_1".to_string(),
        descriptor: catalog::CONTEXT_BUDGET_EXCEEDED,
        scope: ErrorScope::new("strand", "ss_1"),
        source: ErrorSource::new("santi-core", operation),
        message: "over budget".to_string(),
        context: json!({"phase": operation}),
    }
}

#[test]
fn repeat_is_idempotent() {
    let opened = engine().open_incident(None, draft("ingest_admission"), "t1");
    let repeated = engine().open_incident(Some(&opened.incident), draft("active_guard"), "t2");

    assert_eq!(repeated.incident.id, opened.incident.id);
    assert_eq!(repeated.incident.revision, 1);
    assert_eq!(repeated.incident.occurrence_count, 2);
    assert!(repeated.transition.is_none());
    assert_eq!(repeated.incident.source.operation, "ingest_admission");
    assert_eq!(repeated.incident.latest_source.operation, "active_guard");
}

#[test]
fn resolve_advances_revision() {
    let opened = engine().open_incident(None, draft("provider_preflight"), "t1");
    let resolved = engine().resolve_incident(
        &opened.incident,
        "compact_exec",
        json!({"total_bytes": 42}),
        "t2",
    );

    assert_eq!(resolved.incident.status, IncidentStatus::Resolved);
    assert_eq!(resolved.incident.revision, 2);
    assert_eq!(
        resolved.transition.unwrap().kind,
        ErrorTransitionKind::Resolved
    );
}
