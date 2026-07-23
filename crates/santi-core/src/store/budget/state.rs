use crate::store::db::Database;
use santi_provider::ProviderItem;

use crate::Fault;

use super::{Pressure, context_incident_key};
use crate::message;

pub(super) fn pending_items(db: &Database<'_>, strand: &str) -> Result<Vec<ProviderItem>, String> {
    let mut items = Vec::new();
    for (kind, content_json) in db.pending_inbox_rows(strand)? {
        let content = serde_json::from_str::<message::Content>(&content_json)
            .map_err(|error| error.to_string())?;
        if let Some(item) =
            crate::context::budget::inbound_provider_item(&message::Kind::decode(&kind), &content)
        {
            items.push(item);
        }
    }
    Ok(items)
}

pub(super) fn open_context_incident(
    db: &Database<'_>,
    strand: &str,
    input: Pressure<'_>,
) -> Result<Fault, String> {
    db.open_incident(input.into_draft(strand))
}

pub(super) fn repeat_context_incident(
    db: &Database<'_>,
    strand: &str,
    operation: &str,
) -> Result<Fault, String> {
    let key = context_incident_key(strand);
    let existing = db
        .incident(&key)?
        .ok_or_else(|| "active context-budget incident missing".to_string())?;
    db.open_incident(santi_error::Draft {
        key,
        descriptor: santi_error::catalog::CONTEXT_BUDGET_EXCEEDED,
        scope: existing.scope,
        source: santi_error::Source::new("santi-core", operation),
        message: existing.latest.message,
        context: existing.latest.context,
    })
}
