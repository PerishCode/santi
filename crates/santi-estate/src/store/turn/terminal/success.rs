use super::{Store, complete, read};
use crate::store::error;
use crate::store::turn::{OutboxDraft, outbox};
use keel::adapt::db::Sqlite;
use keel::{Op, Row, Tx, form};
use santi_error::Ruled;
use santi_model::{event, turn};

pub struct CompletionDraft<'a> {
    pub turn: &'a str,
    pub reply: Option<&'a str>,
    pub provider: &'a str,
    pub model: &'a str,
    pub response: Option<&'a str>,
    pub occurred: &'a str,
}

pub struct Completion {
    pub turn: turn::Turn,
    pub event: Option<event::Event>,
}

struct Writer<'a, 'tx>(&'a mut Tx<'tx, Sqlite>);

impl Store {
    pub async fn finish_turn(&self, draft: CompletionDraft<'_>) -> Result<Completion, String> {
        let tag = draft.turn.to_string();
        let event = self
            .core
            .batch(async |tx| finish(tx, draft).await)
            .await
            .map_err(read::error)?;
        let turn = self
            .turn(&tag)
            .await?
            .ok_or_else(|| "completed turn missing".to_string())?;
        Ok(Completion { turn, event })
    }
}

async fn finish(
    tx: &mut Tx<'_, Sqlite>,
    draft: CompletionDraft<'_>,
) -> Result<Option<event::Event>, keel::adapt::Error> {
    let turn = tx
        .one(&form("Turn").when("tag", Op::Eq, draft.turn))
        .await?
        .ok_or_else(|| keel::adapt::Error::Missing(draft.turn.into()))?;
    let strand = Writer(tx).relation(&turn, "strand", "turn strand").await?;
    let strand_tag = text(&strand, "tag")?;
    let reply = match draft.reply {
        Some(tag) => Some(reply(tx, &strand, tag).await?),
        None => None,
    };
    let boundary = turn
        .int("from")
        .ok_or_else(|| keel::adapt::Error::Adapt("turn boundary missing".into()))?;
    if reply
        .as_ref()
        .is_some_and(|reply| reply.sequence <= boundary)
    {
        return Err(keel::adapt::Error::Adapt(
            "completion reply does not follow its turn boundary".into(),
        ));
    }
    let to = strand
        .int("next")
        .ok_or_else(|| keel::adapt::Error::Adapt("strand next missing".into()))?
        - 1;
    update_strand(tx, &strand, reply.as_ref(), &draft).await?;
    complete(tx, draft.turn, to, draft.occurred).await?;
    Writer(tx).resolve_success(strand_tag, &draft).await?;
    let event = event(&turn, &strand, reply.as_ref(), draft.occurred)?;
    if let Some(event) = event.as_ref() {
        let payload = serde_json::to_string(event)
            .map_err(|error| keel::adapt::Error::Adapt(error.to_string()))?;
        outbox::queue(
            tx,
            OutboxDraft {
                stream: "turns",
                event,
            },
            &payload,
        )
        .await?;
    }
    Ok(event)
}

struct Reply {
    sequence: i64,
    text: String,
}

async fn reply(
    tx: &mut Tx<'_, Sqlite>,
    strand: &Row,
    tag: &str,
) -> Result<Reply, keel::adapt::Error> {
    let message = tx
        .one(&form("Message").when("tag", Op::Eq, tag))
        .await?
        .ok_or_else(|| keel::adapt::Error::Missing(format!("reply {tag}")))?;
    let entry = tx
        .one(
            &form("StrandEntry")
                .when("target_type", Op::Eq, "message")
                .when("target", Op::Eq, tag),
        )
        .await?
        .ok_or_else(|| keel::adapt::Error::Missing("reply entry".into()))?;
    if entry.int("strand") != Some(strand.key()) {
        return Err(keel::adapt::Error::Adapt(
            "completion reply belongs to another strand".into(),
        ));
    }
    let content = message
        .text("content")
        .ok_or_else(|| keel::adapt::Error::Adapt("reply content missing".into()))?;
    let content = serde_json::from_str::<santi_model::message::Content>(content)
        .map_err(|error| keel::adapt::Error::Adapt(error.to_string()))?;
    Ok(Reply {
        sequence: entry
            .int("sequence")
            .ok_or_else(|| keel::adapt::Error::Adapt("reply sequence missing".into()))?,
        text: content.rendered(),
    })
}

async fn update_strand(
    tx: &mut Tx<'_, Sqlite>,
    strand: &Row,
    reply: Option<&Reply>,
    draft: &CompletionDraft<'_>,
) -> Result<(), keel::adapt::Error> {
    let state = draft
        .response
        .map(|response| {
            serde_json::to_string(&serde_json::json!({
                "provider": draft.provider,
                "opaque": { "response_id": response },
                "schema_version": "santi-v1",
            }))
        })
        .transpose()
        .map_err(|error| keel::adapt::Error::Adapt(error.to_string()))?;
    let seen = reply.map(|reply| reply.sequence.to_string());
    let mut fields = vec![("updated", draft.occurred)];
    if let Some(state) = state.as_deref() {
        fields.push(("state", state));
    }
    if let Some(seen) = seen.as_deref() {
        fields.push(("seen", seen));
    }
    tx.set("Strand", strand.key(), &fields).await?;
    if state.is_none() {
        tx.unset("Strand", strand.key(), &["state"]).await?;
    }
    Ok(())
}

impl Writer<'_, '_> {
    async fn resolve_success(
        &mut self,
        strand: &str,
        draft: &CompletionDraft<'_>,
    ) -> Result<(), keel::adapt::Error> {
        let provider = turn::Error::Provider.descriptor();
        error::resolve_in(
            self.0,
            error::Resolution {
                key: &provider.key("strand", strand),
                by: "provider.turn_succeeded",
                context: serde_json::json!({
                    "turn": draft.turn,
                    "provider": draft.provider,
                    "model": draft.model,
                    "response": draft.response,
                }),
                now: draft.occurred,
            },
        )
        .await?;
        let runtime = turn::Error::Runtime.descriptor();
        error::resolve_in(
            self.0,
            error::Resolution {
                key: &runtime.key("strand", strand),
                by: "runtime.turn_succeeded",
                context: serde_json::json!({
                    "turn": draft.turn,
                    "provider": draft.provider,
                    "model": draft.model,
                }),
                now: draft.occurred,
            },
        )
        .await?;
        let execution = santi_model::budget::Error::Execution.descriptor();
        error::resolve_in(
            self.0,
            error::Resolution {
                key: &execution.key("strand", strand),
                by: "execution_budget.turn_succeeded",
                context: serde_json::json!({
                    "turn": draft.turn,
                    "provider": draft.provider,
                    "model": draft.model,
                }),
                now: draft.occurred,
            },
        )
        .await?;
        Ok(())
    }
}

fn event(
    turn: &Row,
    strand: &Row,
    reply: Option<&Reply>,
    occurred: &str,
) -> Result<Option<event::Event>, keel::adapt::Error> {
    let Some(label) = strand.text("label") else {
        return Ok(None);
    };
    let Some(reply) = reply.filter(|reply| !reply.text.trim().is_empty()) else {
        return Ok(None);
    };
    Ok(Some(event::Event {
        id: santi_model::tag("tev"),
        strand: text(strand, "tag")?.to_string(),
        turn: text(turn, "tag")?.to_string(),
        label: label.to_string(),
        text: reply.text.clone(),
        completed: occurred.to_string(),
    }))
}

impl Writer<'_, '_> {
    async fn relation(
        &mut self,
        row: &Row,
        field: &str,
        missing: &str,
    ) -> Result<Row, keel::adapt::Error> {
        let key = row
            .int(field)
            .ok_or_else(|| keel::adapt::Error::Adapt(format!("{missing} missing")))?;
        self.0
            .one(&form("Strand").when("id", Op::Eq, &key.to_string()))
            .await?
            .ok_or_else(|| keel::adapt::Error::Missing(missing.into()))
    }
}

fn text<'a>(row: &'a Row, field: &str) -> Result<&'a str, keel::adapt::Error> {
    row.text(field)
        .ok_or_else(|| keel::adapt::Error::Adapt(format!("completion {field} missing")))
}
