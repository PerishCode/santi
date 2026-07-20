use std::sync::OnceLock;

use serde_json::Value;
use uuid::Uuid;

use super::*;

#[derive(Debug, Default)]
pub struct ErrorEngine;

static ENGINE: OnceLock<ErrorEngine> = OnceLock::new();

pub fn engine() -> &'static ErrorEngine {
    ENGINE.get_or_init(|| ErrorEngine)
}

impl ErrorEngine {
    pub fn open_incident(
        &self,
        existing: Option<&ErrorIncident>,
        draft: IncidentDraft,
        now: impl Into<String>,
    ) -> IncidentMutation {
        let now = now.into();
        let (incident, transition) = match existing {
            Some(existing) => {
                let mut incident = existing.clone();
                incident.latest_source = draft.source.clone();
                incident.latest_message = draft.message.clone();
                incident.latest_context = draft.context.clone();
                incident.occurrence_count += 1;
                incident.last_seen_at = now;
                (incident, None)
            }
            None => {
                let (incident, transition) = self.open_new(draft, now);
                (incident, Some(transition))
            }
        };
        let error = self.error_from_incident(&incident);
        IncidentMutation {
            incident,
            error,
            transition,
        }
    }

    pub fn resolve_incident(
        &self,
        active: &ErrorIncident,
        resolved_by: impl Into<String>,
        context: Value,
        now: impl Into<String>,
    ) -> IncidentMutation {
        let now = now.into();
        let mut incident = active.clone();
        incident.status = IncidentStatus::Resolved;
        incident.revision += 1;
        incident.last_seen_at = now.clone();
        incident.resolved_at = Some(now.clone());
        incident.resolved_by = Some(resolved_by.into());
        incident.latest_context = context;
        let transition = ErrorTransition {
            id: prefixed_id("error_event"),
            incident_id: incident.id.clone(),
            revision: incident.revision,
            kind: ErrorTransitionKind::Resolved,
            incident: incident.clone(),
            occurred_at: now,
        };
        let error = self.error_from_incident(&incident);
        IncidentMutation {
            incident,
            error,
            transition: Some(transition),
        }
    }

    pub fn transient(&self, signal: Signal) -> SantiError {
        SantiError {
            id: prefixed_id("error"),
            incident_id: None,
            code: signal.descriptor.code.to_string(),
            message: signal.message,
            category: signal.descriptor.category,
            severity: signal.descriptor.severity,
            retry: signal.descriptor.retry,
            exposure: signal.descriptor.exposure,
            source: signal.source,
            scope: signal.scope,
            context: signal.context,
        }
    }

    pub fn dispatch_outbox(
        &self,
        outbox: &impl ErrorOutbox,
        sink: &impl ErrorEventSink,
        limit: usize,
    ) -> Result<usize, String> {
        let transitions = outbox.pending_error_transitions(limit)?;
        let mut delivered = 0;
        for transition in transitions {
            sink.publish_error_transition(&transition)?;
            outbox.mark_error_transition_delivered(&transition.id)?;
            delivered += 1;
        }
        Ok(delivered)
    }

    fn error_from_incident(&self, incident: &ErrorIncident) -> SantiError {
        SantiError {
            id: prefixed_id("error"),
            incident_id: Some(incident.id.clone()),
            code: incident.code.clone(),
            message: incident.latest_message.clone(),
            category: incident.category,
            severity: incident.severity,
            retry: incident.retry,
            exposure: incident.exposure,
            source: incident.latest_source.clone(),
            scope: Some(incident.scope.clone()),
            context: incident.latest_context.clone(),
        }
    }

    fn open_new(&self, draft: IncidentDraft, now: String) -> (ErrorIncident, ErrorTransition) {
        let incident = ErrorIncident {
            id: prefixed_id("inc"),
            incident_key: draft.incident_key,
            code: draft.descriptor.code.to_string(),
            status: IncidentStatus::Active,
            category: draft.descriptor.category,
            severity: draft.descriptor.severity,
            retry: draft.descriptor.retry,
            exposure: draft.descriptor.exposure,
            scope: draft.scope,
            source: draft.source.clone(),
            latest_source: draft.source,
            message: draft.message.clone(),
            latest_message: draft.message,
            context: draft.context.clone(),
            latest_context: draft.context,
            occurrence_count: 1,
            revision: 1,
            first_seen_at: now.clone(),
            last_seen_at: now.clone(),
            resolved_at: None,
            resolved_by: None,
        };
        let transition = ErrorTransition {
            id: prefixed_id("error_event"),
            incident_id: incident.id.clone(),
            revision: incident.revision,
            kind: ErrorTransitionKind::Opened,
            incident: incident.clone(),
            occurred_at: now,
        };
        (incident, transition)
    }
}

fn prefixed_id(prefix: &str) -> String {
    format!("{prefix}_{}", Uuid::new_v4().simple())
}
