use super::{Store, read};
use keel::{Op, Rank, form};
use santi_model::event;

#[derive(Clone, Copy)]
pub struct OutboxDraft<'a> {
    pub stream: &'a str,
    pub event: &'a event::Event,
}

#[derive(PartialEq, Eq)]
struct Fingerprint<'a> {
    tag: &'a str,
    label: &'a str,
    payload: &'a str,
    stream: i64,
}

impl<'a> Fingerprint<'a> {
    fn read(row: &'a keel::Row) -> Option<Self> {
        Some(Self {
            tag: row.text("tag")?,
            label: row.text("label")?,
            payload: row.text("payload")?,
            stream: row.int("stream")?,
        })
    }
}

impl Store {
    pub async fn queue_outbox(&self, draft: OutboxDraft<'_>) -> Result<(), String> {
        let payload = serde_json::to_string(draft.event).map_err(|error| error.to_string())?;
        self.core
            .batch(async |tx| queue(tx, draft, &payload).await)
            .await
            .map_err(read::error)
    }

    pub async fn outbox(
        &self,
        stream: &str,
        after: i64,
        prefix: &str,
        limit: usize,
    ) -> Result<event::Batch, String> {
        let Some(stream) = read::one(&self.core, "OutboxStream", "tag", stream).await? else {
            return Ok(event::Batch {
                cursor: after,
                events: Vec::new(),
            });
        };
        let rows = self
            .core
            .ask(
                &form("TurnOutbox")
                    .when("stream", Op::Eq, &stream.key().to_string())
                    .when("sequence", Op::Gt, &after.to_string())
                    .order("sequence", Rank::Asc),
            )
            .await
            .map_err(read::error)?;
        let crest = rows
            .rows()
            .iter()
            .filter_map(|row| row.int("sequence"))
            .max()
            .unwrap_or(after)
            .max(after);
        let mut selected = rows
            .rows()
            .iter()
            .filter(|row| {
                row.text("label")
                    .is_some_and(|label| label.starts_with(prefix))
            })
            .collect::<Vec<_>>();
        let more = selected.len() > limit;
        selected.truncate(limit);
        let cursor = if more {
            selected
                .last()
                .and_then(|row| row.int("sequence"))
                .unwrap_or(after)
        } else {
            crest
        };
        let events = selected
            .into_iter()
            .map(|row| {
                let payload = read::text(row, "payload")?;
                serde_json::from_str(payload).map_err(|error| error.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(event::Batch { cursor, events })
    }
}

pub(in crate::store) async fn queue(
    tx: &mut keel::Tx<'_, keel::adapt::db::Sqlite>,
    draft: OutboxDraft<'_>,
    payload: &str,
) -> Result<(), keel::adapt::Error> {
    let stream = match tx
        .one(&form("OutboxStream").when("tag", Op::Eq, draft.stream))
        .await?
    {
        Some(stream) => stream.key(),
        None => tx.put("OutboxStream", &[("tag", draft.stream)]).await?,
    };
    let turn = read::need(tx, "Turn", "tag", &draft.event.turn).await?;
    let completion = tx
        .one(&form("TurnCompletion").when("turn", Op::Eq, &turn.to_string()))
        .await?
        .ok_or_else(|| keel::adapt::Error::Adapt("outbox turn is not completed".into()))?;
    if completion.text("finished") != Some(draft.event.completed.as_str()) {
        return Err(keel::adapt::Error::Adapt(
            "outbox event completion does not match its turn".into(),
        ));
    }
    let turn_row = tx
        .one(&form("Turn").when("id", Op::Eq, &turn.to_string()))
        .await?
        .ok_or_else(|| keel::adapt::Error::Missing("outbox turn".into()))?;
    let strand = turn_row
        .int("strand")
        .ok_or_else(|| keel::adapt::Error::Adapt("turn strand missing".into()))?;
    let strand = tx
        .one(&form("Strand").when("id", Op::Eq, &strand.to_string()))
        .await?
        .ok_or_else(|| keel::adapt::Error::Missing("outbox strand".into()))?;
    if strand.text("tag") != Some(draft.event.strand.as_str()) {
        return Err(keel::adapt::Error::Adapt(
            "outbox event belongs to another strand".into(),
        ));
    }
    if strand.text("label") != Some(draft.event.label.as_str()) {
        return Err(keel::adapt::Error::Adapt(
            "outbox event label does not match its strand".into(),
        ));
    }
    if let Some(existing) = tx
        .one(&form("TurnOutbox").when("turn", Op::Eq, &turn.to_string()))
        .await?
    {
        let requested = Fingerprint {
            tag: draft.event.id.as_str(),
            label: draft.event.label.as_str(),
            payload,
            stream,
        };
        if Fingerprint::read(&existing) == Some(requested) {
            return Ok(());
        }
        return Err(keel::adapt::Error::Adapt(
            "turn outbox conflicts with its accepted event".into(),
        ));
    }
    tx.put(
        "TurnOutbox",
        &[
            ("tag", draft.event.id.as_str()),
            ("label", draft.event.label.as_str()),
            ("payload", payload),
            ("created", draft.event.completed.as_str()),
            ("stream", &stream.to_string()),
            ("turn", &turn.to_string()),
        ],
    )
    .await?;
    Ok(())
}
