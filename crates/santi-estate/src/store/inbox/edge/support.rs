use super::{ReplayDraft, WebhookDraft};

pub(super) fn exact(row: &keel::Row, draft: &WebhookDraft<'_>, soul: i64) -> bool {
    row.text("adaptor") == Some(draft.adaptor)
        && row.text("strategy") == Some(draft.strategy)
        && row.text("credential") == Some(draft.credential)
        && row.int("soul") == Some(soul)
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
