use std::sync::OnceLock;

use serde_json::Value;
use uuid::Uuid;

use super::*;

#[derive(Debug, Default)]
pub struct Engine;

static ENGINE: OnceLock<Engine> = OnceLock::new();

pub fn engine() -> &'static Engine {
    ENGINE.get_or_init(|| Engine)
}

impl Engine {
    pub fn open(
        &self,
        existing: Option<&Incident>,
        draft: Draft,
        now: impl Into<String>,
    ) -> Mutation {
        let now = now.into();
        let (incident, transition) = match existing {
            Some(existing) => {
                let mut incident = existing.clone();
                incident.latest = Report {
                    source: draft.source,
                    message: draft.message,
                    context: draft.context,
                    seen: now,
                };
                incident.occurrences += 1;
                (incident, None)
            }
            None => {
                let (incident, transition) = self.raise(draft, now);
                (incident, Some(transition))
            }
        };
        let error = self.fault(&incident);
        Mutation {
            incident,
            error,
            transition,
        }
    }

    pub fn resolve(
        &self,
        active: &Incident,
        by: impl Into<String>,
        context: Value,
        now: impl Into<String>,
    ) -> Mutation {
        let now = now.into();
        let mut incident = active.clone();
        incident.status = Status::Resolved;
        incident.revision += 1;
        incident.latest.seen = now.clone();
        incident.latest.context = context;
        incident.resolution = Some(Resolution {
            at: now.clone(),
            by: Some(by.into()),
        });
        let transition = Transition {
            id: tag("error_event"),
            incident: incident.id.clone(),
            revision: incident.revision,
            kind: Kind::Resolved,
            held: incident.clone(),
            occurred: now,
        };
        let error = self.fault(&incident);
        Mutation {
            incident,
            error,
            transition: Some(transition),
        }
    }

    pub fn transient(&self, signal: Signal) -> Fault {
        Fault {
            id: tag("error"),
            incident: None,
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

    pub fn dispatch(
        &self,
        outbox: &impl Outbox,
        sink: &impl Sink,
        limit: usize,
    ) -> Result<usize, String> {
        let transitions = outbox.pending(limit)?;
        let mut sent = 0;
        for transition in transitions {
            sink.publish(&transition)?;
            outbox.delivered(&transition.id)?;
            sent += 1;
        }
        Ok(sent)
    }

    fn fault(&self, incident: &Incident) -> Fault {
        Fault {
            id: tag("error"),
            incident: Some(incident.id.clone()),
            code: incident.code.clone(),
            message: incident.latest.message.clone(),
            category: incident.category,
            severity: incident.severity,
            retry: incident.retry,
            exposure: incident.exposure,
            source: incident.latest.source.clone(),
            scope: Some(incident.scope.clone()),
            context: incident.latest.context.clone(),
        }
    }

    fn raise(&self, draft: Draft, now: String) -> (Incident, Transition) {
        let incident = Incident {
            id: tag("inc"),
            key: draft.key,
            code: draft.descriptor.code.to_string(),
            status: Status::Active,
            category: draft.descriptor.category,
            severity: draft.descriptor.severity,
            retry: draft.descriptor.retry,
            exposure: draft.descriptor.exposure,
            scope: draft.scope,
            first: Report {
                source: draft.source.clone(),
                message: draft.message.clone(),
                context: draft.context.clone(),
                seen: now.clone(),
            },
            latest: Report {
                source: draft.source,
                message: draft.message,
                context: draft.context,
                seen: now.clone(),
            },
            occurrences: 1,
            revision: 1,
            resolution: None,
        };
        let transition = Transition {
            id: tag("error_event"),
            incident: incident.id.clone(),
            revision: incident.revision,
            kind: Kind::Opened,
            held: incident.clone(),
            occurred: now,
        };
        (incident, transition)
    }
}

fn tag(prefix: &str) -> String {
    format!("{prefix}_{}", Uuid::new_v4().simple())
}
