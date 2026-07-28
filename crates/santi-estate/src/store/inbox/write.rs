use super::{InboxDraft, NoticeDraft, Offer, json, kind};
use keel::adapt::db::Sqlite;
use keel::{Row, Tx};
use std::collections::BTreeSet;

pub(in crate::store) async fn accept(
    tx: &mut Tx<'_, Sqlite>,
    draft: &InboxDraft<'_>,
    gate: usize,
    content: &str,
    metadata: Option<&str>,
) -> Result<(), keel::adapt::Error> {
    let strand = crate::store::read::need(tx, "Strand", "tag", draft.strand).await?;
    capacity(tx, strand, gate).await?;
    let strand = strand.to_string();
    let mut fields = vec![
        ("tag", draft.tag),
        ("kind", kind(&draft.kind)),
        ("content", content),
        ("created", draft.created),
        ("strand", strand.as_str()),
    ];
    if let Some(source) = draft.source {
        fields.push(("source_type", source.kind.as_str()));
        if let Some(reference) = source.source.as_deref() {
            fields.push(("source_ref", reference));
        }
        if let Some(metadata) = metadata {
            fields.push(("source_metadata", metadata));
        }
    }
    tx.put("StrandInbox", &fields).await?;
    super::receipt::accept(tx, draft.tag, &strand, draft.created).await
}

pub(in crate::store) async fn offer(
    tx: &mut Tx<'_, Sqlite>,
    draft: NoticeDraft<'_>,
    gate: usize,
) -> Result<Offer, keel::adapt::Error> {
    if draft.revision < 1 {
        return Err(adapt("inbox notice revision must be positive".into()));
    }
    let content = json(draft.content).map_err(adapt)?;
    let metadata = draft
        .source
        .metadata
        .as_ref()
        .map(json)
        .transpose()
        .map_err(adapt)?;
    let causes = draft.causes.iter().cloned().collect::<BTreeSet<_>>();
    let strand = crate::store::read::need(tx, "Strand", "tag", draft.strand).await?;
    let strand_key = strand.to_string();
    let slot = tx
        .one(
            &keel::form("InboxSlot")
                .when("strand", keel::Op::Eq, &strand_key)
                .when("key", keel::Op::Eq, draft.key),
        )
        .await?;
    if let Some(slot) = slot.as_ref()
        && let Some(offer) = settle(tx, slot, draft, &content, metadata.as_deref(), &causes).await?
    {
        return Ok(offer);
    }
    capacity(tx, strand, gate).await?;
    let causes = json(&causes).map_err(adapt)?;
    let inbox = put_notice(
        tx,
        draft,
        &strand_key,
        &content,
        metadata.as_deref(),
        &causes,
    )
    .await?;
    super::receipt::accept(tx, draft.tag, &strand_key, draft.created).await?;
    match slot {
        Some(slot) => update_slot(tx, &slot, draft, inbox).await?,
        None => put_slot(tx, draft, &strand_key, inbox).await?,
    }
    Ok(Offer {
        inbox: Some(draft.tag.to_string()),
        inserted: true,
    })
}

pub(super) async fn capacity(
    tx: &mut Tx<'_, Sqlite>,
    strand: i64,
    gate: usize,
) -> Result<(), keel::adapt::Error> {
    let pending = tx
        .ask(&keel::form("StrandInbox").when("strand", keel::Op::Eq, &strand.to_string()))
        .await?
        .rows()
        .len();
    if pending < gate {
        Ok(())
    } else {
        Err(keel::adapt::Error::Adapt(format!(
            "strand inbox is full ({pending} pending, gate {gate})"
        )))
    }
}

pub(super) async fn settle(
    tx: &mut Tx<'_, Sqlite>,
    slot: &Row,
    draft: NoticeDraft<'_>,
    content: &str,
    metadata: Option<&str>,
    incoming: &BTreeSet<String>,
) -> Result<Option<Offer>, keel::adapt::Error> {
    let current = slot
        .int("revision")
        .ok_or_else(|| keel::adapt::Error::Adapt("slot revision missing".into()))?;
    let inbox = linked(tx, slot).await?;
    if draft.revision < current {
        return Ok(Some(held(inbox.as_ref())?));
    }
    if draft.revision == current {
        if slot.text("digest") != Some(draft.digest) {
            return Err(keel::adapt::Error::Adapt(
                "inbox notice revision conflicts with its accepted payload".into(),
            ));
        }
        return Ok(Some(held(inbox.as_ref())?));
    }
    let Some(inbox) = inbox else {
        return Ok(None);
    };
    let causes = merged(&inbox, incoming)?;
    update_notice(tx, &inbox, draft, content, metadata, &causes).await?;
    update_slot(tx, slot, draft, inbox.key()).await?;
    Ok(Some(Offer {
        inbox: Some(inbox_tag(&inbox)?),
        inserted: false,
    }))
}

pub(super) async fn put_notice(
    tx: &mut Tx<'_, Sqlite>,
    draft: NoticeDraft<'_>,
    strand: &str,
    content: &str,
    metadata: Option<&str>,
    causes: &str,
) -> Result<i64, keel::adapt::Error> {
    let revision = draft.revision.to_string();
    let mut fields = vec![
        ("tag", draft.tag),
        ("kind", "santi_system"),
        ("content", content),
        ("source_type", draft.source.kind.as_str()),
        ("coalesce_key", draft.key),
        ("coalesce_revision", revision.as_str()),
        ("coalesce_causes", causes),
        ("created", draft.created),
        ("strand", strand),
    ];
    optional_source(&mut fields, draft.source.source.as_deref(), metadata);
    tx.put("StrandInbox", &fields).await
}

pub(super) async fn update_notice(
    tx: &mut Tx<'_, Sqlite>,
    inbox: &Row,
    draft: NoticeDraft<'_>,
    content: &str,
    metadata: Option<&str>,
    causes: &BTreeSet<String>,
) -> Result<(), keel::adapt::Error> {
    if inbox.text("coalesce_key") != Some(draft.key) {
        return Err(keel::adapt::Error::Adapt(
            "inbox slot points to a mismatched pending item".into(),
        ));
    }
    let causes = json(causes).map_err(adapt)?;
    let revision = draft.revision.to_string();
    let mut fields = vec![
        ("content", content),
        ("source_type", draft.source.kind.as_str()),
        ("coalesce_revision", revision.as_str()),
        ("coalesce_causes", causes.as_str()),
    ];
    optional_source(&mut fields, draft.source.source.as_deref(), metadata);
    tx.set("StrandInbox", inbox.key(), &fields).await?;
    let mut absent = Vec::new();
    if draft.source.source.is_none() {
        absent.push("source_ref");
    }
    if draft.source.metadata.is_none() {
        absent.push("source_metadata");
    }
    if !absent.is_empty() {
        tx.unset("StrandInbox", inbox.key(), &absent).await?;
    }
    Ok(())
}

pub(super) async fn put_slot(
    tx: &mut Tx<'_, Sqlite>,
    draft: NoticeDraft<'_>,
    strand: &str,
    inbox: i64,
) -> Result<(), keel::adapt::Error> {
    tx.put(
        "InboxSlot",
        &[
            ("key", draft.key),
            ("revision", &draft.revision.to_string()),
            ("digest", draft.digest),
            ("updated", draft.created),
            ("strand", strand),
            ("inbox", &inbox.to_string()),
        ],
    )
    .await?;
    Ok(())
}

pub(super) async fn update_slot(
    tx: &mut Tx<'_, Sqlite>,
    slot: &Row,
    draft: NoticeDraft<'_>,
    inbox: i64,
) -> Result<(), keel::adapt::Error> {
    tx.set(
        "InboxSlot",
        slot.key(),
        &[
            ("revision", &draft.revision.to_string()),
            ("digest", draft.digest),
            ("updated", draft.created),
            ("inbox", &inbox.to_string()),
        ],
    )
    .await
}

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

async fn linked(tx: &mut Tx<'_, Sqlite>, slot: &Row) -> Result<Option<Row>, keel::adapt::Error> {
    let Some(key) = slot.int("inbox") else {
        return Ok(None);
    };
    tx.one(&keel::form("StrandInbox").when("id", keel::Op::Eq, &key.to_string()))
        .await?
        .map(Some)
        .ok_or_else(|| keel::adapt::Error::Missing("slot inbox".into()))
}

fn held(inbox: Option<&Row>) -> Result<Offer, keel::adapt::Error> {
    Ok(Offer {
        inbox: inbox.map(inbox_tag).transpose()?,
        inserted: false,
    })
}

fn inbox_tag(row: &Row) -> Result<String, keel::adapt::Error> {
    row.text("tag")
        .map(str::to_string)
        .ok_or_else(|| keel::adapt::Error::Adapt("inbox tag missing".into()))
}

fn optional_source<'a>(
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

fn adapt(error: String) -> keel::adapt::Error {
    keel::adapt::Error::Adapt(error)
}
