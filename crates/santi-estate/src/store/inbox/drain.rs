use super::{Begun, DrainDraft, Opening, Store, read, receipt};
use crate::store::write;
use keel::adapt::db::Sqlite;
use keel::{Op, Rank, Row, Tx, form};
use santi_model::receipt as receipt_model;
use serde_json::{Value, json};

mod codec;
mod types;
use codec::{Pending, aggregate, decode, trigger};
use types::{Assigned, Message, Opened, Written};

struct Writer<'a, 'tx>(&'a mut Tx<'tx, Sqlite>);

pub(super) async fn open(store: &Store, draft: DrainDraft<'_>) -> Result<Opening, String> {
    let turn = draft.turn.to_string();
    let opened = store
        .core
        .batch(async |tx| open_in(tx, draft).await)
        .await
        .map_err(read::error)?;
    match opened {
        Opened::Idle => Ok(Opening::Idle),
        Opened::Running(tag) => {
            let turn = store
                .turn(&tag)
                .await?
                .ok_or_else(|| "running turn missing".to_string())?;
            Ok(Opening::Running(turn))
        }
        Opened::Started(messages) => {
            let turn = store
                .turn(&turn)
                .await?
                .ok_or_else(|| "started turn missing".to_string())?;
            let mut drained = Vec::with_capacity(messages.len());
            for tag in messages {
                drained.push(
                    store
                        .message(&tag)
                        .await?
                        .ok_or_else(|| format!("drained message {tag} missing"))?,
                );
            }
            Ok(Opening::Started(Begun { turn, drained }))
        }
    }
}

async fn open_in(
    tx: &mut Tx<'_, Sqlite>,
    draft: DrainDraft<'_>,
) -> Result<Opened, keel::adapt::Error> {
    let strand = tx
        .one(&form("Strand").when("tag", Op::Eq, draft.strand))
        .await?
        .ok_or_else(|| keel::adapt::Error::Missing(draft.strand.into()))?;
    if let Some(tag) = running(tx, strand.key()).await? {
        return Ok(Opened::Running(tag));
    }
    let pending = pending(tx, strand.key()).await?;
    if pending.is_empty() {
        return Ok(Opened::Idle);
    }
    let (notices, regular): (Vec<_>, Vec<_>) = pending
        .into_iter()
        .partition(|pending| pending.coalesce_key.is_some());
    let mut messages = Vec::new();
    let mut assigned = Vec::new();
    let mut writer = Writer(tx);
    for pending in regular {
        let message = writer
            .insert(Message {
                strand: &strand,
                kind: &pending.kind,
                content: &pending.content,
                draft: &draft,
            })
            .await?;
        messages.push(message.clone());
        assigned.push(Assigned { pending, message });
    }
    if !notices.is_empty() {
        let content = aggregate(&notices, draft.created)?;
        let message = writer
            .insert(Message {
                strand: &strand,
                kind: "santi_system",
                content: &content,
                draft: &draft,
            })
            .await?;
        for pending in notices {
            assigned.push(Assigned {
                pending,
                message: message.clone(),
            });
        }
        messages.push(message);
    }
    let from = messages
        .last()
        .ok_or_else(|| keel::adapt::Error::Adapt("drain produced no messages".into()))?
        .sequence;
    writer.put_turn(&strand, &draft, from).await?;
    for item in assigned {
        writer.consume(item, &draft).await?;
    }
    Ok(Opened::Started(
        messages.into_iter().map(|message| message.tag).collect(),
    ))
}

async fn running(
    tx: &mut Tx<'_, Sqlite>,
    strand: i64,
) -> Result<Option<String>, keel::adapt::Error> {
    let turns = tx
        .ask(
            &form("Turn")
                .when("strand", Op::Eq, &strand.to_string())
                .order("created", Rank::Desc)
                .order("tag", Rank::Desc),
        )
        .await?;
    for turn in turns.rows() {
        let key = turn.key().to_string();
        let completed = tx
            .one(&form("TurnCompletion").when("turn", Op::Eq, &key))
            .await?
            .is_some();
        let failed = tx
            .one(&form("TurnFailure").when("turn", Op::Eq, &key))
            .await?
            .is_some();
        if !completed && !failed {
            return turn
                .text("tag")
                .map(str::to_string)
                .map(Some)
                .ok_or_else(|| keel::adapt::Error::Adapt("turn tag missing".into()));
        }
    }
    Ok(None)
}

async fn pending(tx: &mut Tx<'_, Sqlite>, strand: i64) -> Result<Vec<Pending>, keel::adapt::Error> {
    let rows = tx
        .ask(
            &form("StrandInbox")
                .when("strand", Op::Eq, &strand.to_string())
                .order("created", Rank::Asc)
                .order("tag", Rank::Asc),
        )
        .await?;
    rows.rows().iter().map(decode).collect()
}

impl Writer<'_, '_> {
    async fn insert(&mut self, message: Message<'_, '_>) -> Result<Written, keel::adapt::Error> {
        let tag = santi_model::tag("msg");
        let key = self
            .0
            .put(
                "Message",
                &[
                    ("tag", tag.as_str()),
                    ("actor_type", "system"),
                    ("actor", message.draft.actor),
                    ("kind", message.kind),
                    ("content", message.content),
                    ("state", "fixed"),
                    ("request", "true"),
                    ("created", message.draft.created),
                    ("updated", message.draft.created),
                ],
            )
            .await?;
        let strand = self
            .0
            .one(&form("Strand").when("id", Op::Eq, &message.strand.key().to_string()))
            .await?
            .ok_or_else(|| keel::adapt::Error::Missing("drain strand".into()))?;
        let sequence = write::append(
            self.0,
            write::Entry {
                strand: &strand,
                kind: "message",
                target: &tag,
                created: message.draft.created,
            },
        )
        .await?;
        Ok(Written { key, tag, sequence })
    }

    async fn put_turn(
        &mut self,
        strand: &Row,
        draft: &DrainDraft<'_>,
        from: i64,
    ) -> Result<(), keel::adapt::Error> {
        let strand = strand.key().to_string();
        let from = from.to_string();
        let mut fields = vec![
            ("tag", draft.turn),
            ("trigger", trigger(&draft.trigger)),
            ("from", from.as_str()),
            ("created", draft.created),
            ("updated", draft.created),
            ("strand", strand.as_str()),
        ];
        if let Some(source) = draft.source {
            fields.push(("source", source));
        }
        self.0.put("Turn", &fields).await?;
        Ok(())
    }

    async fn consume(
        &mut self,
        item: Assigned,
        draft: &DrainDraft<'_>,
    ) -> Result<(), keel::adapt::Error> {
        let metadata = item
            .pending
            .source_metadata
            .as_deref()
            .map(serde_json::from_str::<Value>)
            .transpose()
            .map_err(|error| keel::adapt::Error::Adapt(error.to_string()))?;
        let payload = json!({
            "kind": "inbox_drain",
            "inbox": item.pending.tag,
            "queued": item.pending.created,
            "drained_at": draft.created,
            "committing_turn_id": draft.turn,
            "message": item.message.tag,
            "seq": item.message.sequence,
            "source": {
                "type": item.pending.source_type,
                "ref": item.pending.source_ref,
                "metadata": metadata,
            }
        })
        .to_string();
        self.0
            .put(
                "MessageEvent",
                &[
                    ("tag", &santi_model::tag("mev")),
                    ("action", "insert"),
                    ("actor_type", "system"),
                    ("actor", draft.actor),
                    ("base_version", "1"),
                    ("payload", &payload),
                    ("created", draft.created),
                    ("message", &item.message.key.to_string()),
                ],
            )
            .await?;
        if let Some(slot) = self
            .0
            .one(&form("InboxSlot").when("inbox", Op::Eq, &item.pending.key.to_string()))
            .await?
        {
            self.0.unset("InboxSlot", slot.key(), &["inbox"]).await?;
            self.0
                .set("InboxSlot", slot.key(), &[("updated", draft.created)])
                .await?;
        }
        receipt::shift(
            self.0,
            super::ReceiptDraft {
                inbox: &item.pending.tag,
                state: receipt_model::State::Driving,
                turn: Some(draft.turn),
                incident: None,
                rebuilt: None,
                occurred: draft.created,
            },
        )
        .await?;
        self.0.end("StrandInbox", item.pending.key).await?;
        Ok(())
    }
}
