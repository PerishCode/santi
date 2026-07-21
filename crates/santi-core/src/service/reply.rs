use santi_protocol::{ReplyEvent, ReplySink};

use super::Service;

pub(super) struct Sink<'a> {
    pub(super) service: &'a Service,
}

impl ReplySink for Sink<'_> {
    fn deliver_reply(&self, event: &ReplyEvent) -> Result<(), String> {
        self.service.store.deliver_reply(event)
    }
}

impl Service {
    pub(in crate::service) fn dispatch_replies(&self) {
        let sink = Sink { service: self };
        if let Err(error) = santi_protocol::dispatch_replies(&self.store, &sink, 256) {
            eprintln!("santi: reply outbox dispatch failed: {error}");
        }
    }
}
