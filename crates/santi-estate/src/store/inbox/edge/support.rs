use super::{ReplayDraft, WebhookDraft};

#[derive(PartialEq, Eq)]
struct Identity<'a> {
    adaptor: &'a str,
    strategy: &'a str,
    credential: &'a str,
    soul: i64,
}

impl<'a> Identity<'a> {
    fn read(row: &'a keel::Row) -> Option<Self> {
        Some(Self {
            adaptor: row.text("adaptor")?,
            strategy: row.text("strategy")?,
            credential: row.text("credential")?,
            soul: row.int("soul")?,
        })
    }
}

pub(super) fn exact(row: &keel::Row, draft: &WebhookDraft<'_>, soul: i64) -> bool {
    Identity::read(row)
        == Some(Identity {
            adaptor: draft.adaptor,
            strategy: draft.strategy,
            credential: draft.credential,
            soul,
        })
}

pub(super) fn conflict(replay: ReplayDraft<'_>) -> &'static str {
    match replay {
        ReplayDraft::Webhook { .. } => "webhook delivery conflicts with an accepted payload",
        ReplayDraft::Downstream { .. } => "downstream request conflicts with an accepted payload",
    }
}

pub(super) fn text(row: &keel::Row, field: &str) -> Result<String, keel::adapt::Error> {
    row.text(field)
        .map(str::to_string)
        .ok_or_else(|| keel::adapt::Error::Adapt(format!("replay {field} missing")))
}
