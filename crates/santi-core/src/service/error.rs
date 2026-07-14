use santi_error::ErrorEventSink;

use crate::{ErrorTransition, SantiStreamPayload};

use super::Service;

pub(super) const NO_ERROR_EVENT_SUBSCRIBERS: &str = "error event bus has no subscribers";

pub(super) struct Sink<'a> {
    pub(super) service: &'a Service,
}

impl ErrorEventSink for Sink<'_> {
    fn publish_error_transition(&self, transition: &ErrorTransition) -> Result<(), String> {
        let strand_delivered = transition.incident.scope.kind == "strand"
            && self
                .service
                .send_stream(
                    &transition.incident.scope.id,
                    SantiStreamPayload::ErrorTransition {
                        transition: Box::new(transition.clone()),
                    },
                )
                .is_ok();
        let global_delivered = self.service.error_events.send(transition.clone()).is_ok();
        if strand_delivered || global_delivered {
            Ok(())
        } else {
            Err(NO_ERROR_EVENT_SUBSCRIBERS.to_string())
        }
    }
}
