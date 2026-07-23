use crate::Transition;

use super::Service;
use crate::stream;

pub(in crate::service) const NO_ERROR_EVENT_SUBSCRIBERS: &str =
    "error event bus has no subscribers";

pub(in crate::service) struct Sink<'a> {
    pub(in crate::service) service: &'a Service,
}

impl santi_error::Sink for Sink<'_> {
    fn publish(&self, transition: &Transition) -> Result<(), String> {
        let strand_delivered = transition.held.scope.kind == "strand"
            && self
                .service
                .send_stream(
                    &transition.held.scope.id,
                    stream::Payload::Transition {
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
