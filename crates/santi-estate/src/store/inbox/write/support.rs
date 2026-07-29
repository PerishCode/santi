use super::super::Offer;
use keel::adapt::db::Sqlite;
use keel::{Row, Tx};
use std::collections::BTreeSet;

pub(super) fn merged(
    inbox: &Row,
    incoming: &BTreeSet<String>,
) -> Result<BTreeSet<String>, keel::adapt::Error> {
    let mut causes: BTreeSet<String> = inbox
        .text("coalesce_causes")
        .map(serde_json::from_str)
        .transpose()
        .map_err(|error| keel::adapt::Error::Adapt(error.to_string()))?
        .unwrap_or_default();
    causes.extend(incoming.iter().cloned());
    Ok(causes)
}

pub(super) async fn linked(
    tx: &mut Tx<'_, Sqlite>,
    slot: &Row,
) -> Result<Option<Row>, keel::adapt::Error> {
    let Some(key) = slot.int("inbox") else {
        return Ok(None);
    };
    tx.one(&keel::form("StrandInbox").when("id", keel::Op::Eq, &key.to_string()))
        .await?
        .map(Some)
        .ok_or_else(|| keel::adapt::Error::Missing("slot inbox".into()))
}

pub(super) fn held(inbox: Option<&Row>) -> Result<Offer, keel::adapt::Error> {
    Ok(Offer {
        inbox: inbox.map(inbox_tag).transpose()?,
        inserted: false,
    })
}

pub(super) fn inbox_tag(row: &Row) -> Result<String, keel::adapt::Error> {
    row.text("tag")
        .map(str::to_string)
        .ok_or_else(|| keel::adapt::Error::Adapt("inbox tag missing".into()))
}

pub(super) fn optional_source<'a>(
    fields: &mut Vec<(&'a str, &'a str)>,
    reference: Option<&'a str>,
    metadata: Option<&'a str>,
) {
    if let Some(reference) = reference {
        fields.push(("source_ref", reference));
    }
    if let Some(metadata) = metadata {
        fields.push(("source_metadata", metadata));
    }
}

pub(super) fn adapt(error: String) -> keel::adapt::Error {
    keel::adapt::Error::Adapt(error)
}
