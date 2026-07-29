use super::{InboxDraft, NoticeDraft, Offer, json, kind};
use keel::adapt::db::Sqlite;
use keel::{Row, Tx};
use std::collections::BTreeSet;

mod support;
use support::{adapt, held, inbox_tag, linked, merged, optional_source};

pub(in crate::store) struct Acceptance<'a, 'draft> {
    pub draft: &'a InboxDraft<'draft>,
    pub gate: usize,
    pub content: &'a str,
    pub metadata: Option<&'a str>,
}

#[derive(Clone, Copy)]
struct Notice<'a, 'draft> {
    draft: NoticeDraft<'draft>,
    content: &'a str,
    metadata: Option<&'a str>,
}

struct Writer<'a, 'tx> {
    tx: &'a mut Tx<'tx, Sqlite>,
}

pub(in crate::store) async fn accept(
    tx: &mut Tx<'_, Sqlite>,
    acceptance: Acceptance<'_, '_>,
) -> Result<(), keel::adapt::Error> {
    let draft = acceptance.draft;
    let mut writer = Writer { tx };
    let strand = crate::store::read::need(writer.tx, "Strand", "tag", draft.strand).await?;
    writer.capacity(strand, acceptance.gate).await?;
    let strand = strand.to_string();
    let mut fields = vec![
        ("tag", draft.tag),
        ("kind", kind(&draft.kind)),
        ("content", acceptance.content),
        ("created", draft.created),
        ("strand", strand.as_str()),
    ];
    if let Some(source) = draft.source {
        fields.push(("source_type", source.kind.as_str()));
        if let Some(reference) = source.source.as_deref() {
            fields.push(("source_ref", reference));
        }
        if let Some(metadata) = acceptance.metadata {
            fields.push(("source_metadata", metadata));
        }
    }
    writer.tx.put("StrandInbox", &fields).await?;
    super::receipt::accept(writer.tx, draft.tag, &strand, draft.created).await
}

pub(in crate::store) async fn offer(
    tx: &mut Tx<'_, Sqlite>,
    draft: NoticeDraft<'_>,
    gate: usize,
) -> Result<Offer, keel::adapt::Error> {
    let mut writer = Writer { tx };
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
    let strand = crate::store::read::need(writer.tx, "Strand", "tag", draft.strand).await?;
    let strand_key = strand.to_string();
    let slot = writer
        .tx
        .one(
            &keel::form("InboxSlot")
                .when("strand", keel::Op::Eq, &strand_key)
                .when("key", keel::Op::Eq, draft.key),
        )
        .await?;
    if let Some(slot) = slot.as_ref()
        && let Some(offer) = writer
            .settle(
                slot,
                Notice {
                    draft,
                    content: &content,
                    metadata: metadata.as_deref(),
                },
                &causes,
            )
            .await?
    {
        return Ok(offer);
    }
    writer.capacity(strand, gate).await?;
    let causes = json(&causes).map_err(adapt)?;
    let notice = Notice {
        draft,
        content: &content,
        metadata: metadata.as_deref(),
    };
    let inbox = writer.put_notice(&strand_key, notice, &causes).await?;
    super::receipt::accept(writer.tx, draft.tag, &strand_key, draft.created).await?;
    match slot {
        Some(slot) => writer.update_slot(&slot, draft, inbox).await?,
        None => writer.put_slot(draft, &strand_key, inbox).await?,
    }
    Ok(Offer {
        inbox: Some(draft.tag.to_string()),
        inserted: true,
    })
}

impl Writer<'_, '_> {
    async fn capacity(&mut self, strand: i64, gate: usize) -> Result<(), keel::adapt::Error> {
        let pending = self
            .tx
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

    async fn settle(
        &mut self,
        slot: &Row,
        notice: Notice<'_, '_>,
        incoming: &BTreeSet<String>,
    ) -> Result<Option<Offer>, keel::adapt::Error> {
        let draft = notice.draft;
        let current = slot
            .int("revision")
            .ok_or_else(|| keel::adapt::Error::Adapt("slot revision missing".into()))?;
        let inbox = linked(self.tx, slot).await?;
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
        self.update_notice(&inbox, notice, &causes).await?;
        self.update_slot(slot, draft, inbox.key()).await?;
        Ok(Some(Offer {
            inbox: Some(inbox_tag(&inbox)?),
            inserted: false,
        }))
    }

    async fn put_notice(
        &mut self,
        strand: &str,
        notice: Notice<'_, '_>,
        causes: &str,
    ) -> Result<i64, keel::adapt::Error> {
        let draft = notice.draft;
        let revision = draft.revision.to_string();
        let mut fields = vec![
            ("tag", draft.tag),
            ("kind", "santi_system"),
            ("content", notice.content),
            ("source_type", draft.source.kind.as_str()),
            ("coalesce_key", draft.key),
            ("coalesce_revision", revision.as_str()),
            ("coalesce_causes", causes),
            ("created", draft.created),
            ("strand", strand),
        ];
        optional_source(&mut fields, draft.source.source.as_deref(), notice.metadata);
        self.tx.put("StrandInbox", &fields).await
    }

    async fn update_notice(
        &mut self,
        inbox: &Row,
        notice: Notice<'_, '_>,
        causes: &BTreeSet<String>,
    ) -> Result<(), keel::adapt::Error> {
        let draft = notice.draft;
        if inbox.text("coalesce_key") != Some(draft.key) {
            return Err(keel::adapt::Error::Adapt(
                "inbox slot points to a mismatched pending item".into(),
            ));
        }
        let causes = json(causes).map_err(adapt)?;
        let revision = draft.revision.to_string();
        let mut fields = vec![
            ("content", notice.content),
            ("source_type", draft.source.kind.as_str()),
            ("coalesce_revision", revision.as_str()),
            ("coalesce_causes", causes.as_str()),
        ];
        optional_source(&mut fields, draft.source.source.as_deref(), notice.metadata);
        self.tx.set("StrandInbox", inbox.key(), &fields).await?;
        let mut absent = Vec::new();
        if draft.source.source.is_none() {
            absent.push("source_ref");
        }
        if draft.source.metadata.is_none() {
            absent.push("source_metadata");
        }
        if !absent.is_empty() {
            self.tx.unset("StrandInbox", inbox.key(), &absent).await?;
        }
        Ok(())
    }

    async fn put_slot(
        &mut self,
        draft: NoticeDraft<'_>,
        strand: &str,
        inbox: i64,
    ) -> Result<(), keel::adapt::Error> {
        self.tx
            .put(
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

    async fn update_slot(
        &mut self,
        slot: &Row,
        draft: NoticeDraft<'_>,
        inbox: i64,
    ) -> Result<(), keel::adapt::Error> {
        self.tx
            .set(
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
}
