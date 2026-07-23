use super::super::{Service, address::Address, timing};
use crate::{message, stream, thinking, turn};

pub(in crate::service) struct Update<'a, 'turn> {
    pub(in crate::service) address: Address<&'a str>,
    pub(in crate::service) prose: &'a mut String,
    pub(in crate::service) speech: &'a mut String,
    pub(in crate::service) timing: &'a timing::Turn<'turn>,
    pub(in crate::service) round: usize,
    pub(in crate::service) span: &'a mut Option<thinking::Span>,
    pub(in crate::service) active: &'a Option<String>,
}

impl Service {
    pub(in crate::service) fn spoken(
        &self,
        delta: String,
        update: Update<'_, '_>,
    ) -> Result<(), String> {
        if update.prose.is_empty() {
            update.timing.uttered(update.round);
            self.conclude(
                update.address.strand,
                update.span,
                thinking::Reason::FirstTextDelta,
            )?;
            self.stirred(
                update.address.strand,
                update.address.turn,
                turn::Motion::Generating,
                update.active.clone(),
            );
        }
        update.prose.push_str(&delta);
        update.speech.push_str(&delta);
        self.publish(
            update.address.strand,
            stream::Payload::MessageDelta {
                message: format!("stream_{}", update.address.turn),
                turn: update.address.turn.to_string(),
                role: message::Role::Soul,
                text: delta,
            },
        );
        Ok(())
    }
}
