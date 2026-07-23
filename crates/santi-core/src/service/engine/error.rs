use crate::Transition;

use super::Service;
use crate::stream;

pub(in crate::service) const UNHEARD: &str = "error event bus has no subscribers";

pub(in crate::service) struct Sink<'a> {
    pub(in crate::service) service: &'a Service,
}

impl santi_error::Sink for Sink<'_> {
    fn publish(&self, transition: &Transition) -> Result<(), String> {
        let reached = transition.held.scope.kind == "strand"
            && self
                .service
                .streamed(
                    &transition.held.scope.id,
                    stream::Payload::Transition {
                        transition: Box::new(transition.clone()),
                    },
                )
                .is_ok();
        let delivered = self.service.errors.send(transition.clone()).is_ok();
        if reached || delivered {
            Ok(())
        } else {
            Err(UNHEARD.to_string())
        }
    }
}
