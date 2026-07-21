use santi_model::ImDeliveryMode;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplyEvent {
    pub id: String,
    pub strand_id: String,
    pub turn_id: String,
    pub participant_id: String,
    pub message_id: Option<String>,
    pub content: String,
    pub mode: ImDeliveryMode,
}

pub trait ReplyOutbox {
    fn pending_replies(&self, limit: usize) -> Result<Vec<ReplyEvent>, String>;
    fn mark_reply_delivered(&self, id: &str) -> Result<(), String>;
}

pub trait ReplySink {
    fn deliver_reply(&self, event: &ReplyEvent) -> Result<(), String>;
}

pub fn dispatch_replies(
    outbox: &impl ReplyOutbox,
    sink: &impl ReplySink,
    limit: usize,
) -> Result<usize, String> {
    let pending = outbox.pending_replies(limit)?;
    let mut delivered = 0;
    for event in pending {
        sink.deliver_reply(&event)?;
        outbox.mark_reply_delivered(&event.id)?;
        delivered += 1;
    }
    Ok(delivered)
}
