use super::{Store, read};
use keel::{Op, Rank, form};
use santi_model::turn;

mod outbox;
mod stop;
mod terminal;

pub use outbox::OutboxDraft;
pub use terminal::{
    ClassifiedFailure, ClassifiedFailureDraft, Completion, CompletionDraft, Interruption,
    InterruptionDraft,
};

pub struct TurnDraft<'a> {
    pub tag: &'a str,
    pub strand: &'a str,
    pub trigger: turn::Trigger,
    pub source: Option<&'a str>,
    pub from: i64,
    pub created: &'a str,
}

impl Store {
    pub async fn create_turn(&self, draft: TurnDraft<'_>) -> Result<turn::Turn, String> {
        let tag = draft.tag.to_string();
        self.core
            .batch(async |tx| {
                let strand = read::need(tx, "Strand", "tag", draft.strand).await?;
                let strand = strand.to_string();
                let from = draft.from.to_string();
                let mut fields = vec![
                    ("tag", draft.tag),
                    ("trigger", trigger(&draft.trigger)),
                    ("from", from.as_str()),
                    ("created", draft.created),
                    ("updated", draft.created),
                    ("strand", strand.as_str()),
                ];
                if let Some(source) = draft.source {
                    fields.push(("source", source));
                }
                tx.put("Turn", &fields).await?;
                Ok(())
            })
            .await
            .map_err(read::error)?;
        self.turn(&tag)
            .await?
            .ok_or_else(|| "created turn missing".to_string())
    }

    pub async fn turn(&self, tag: &str) -> Result<Option<turn::Turn>, String> {
        let Some(row) = read::one(&self.core, "Turn", "tag", tag).await? else {
            return Ok(None);
        };
        self.turned(&row).await.map(Some)
    }

    pub async fn turns(&self, strand: &str) -> Result<Vec<turn::Turn>, String> {
        let strand = read::one(&self.core, "Strand", "tag", strand)
            .await?
            .ok_or_else(|| "strand not found".to_string())?;
        let rows = self
            .core
            .ask(
                &form("Turn")
                    .when("strand", Op::Eq, &strand.key().to_string())
                    .order("created", Rank::Asc)
                    .order("tag", Rank::Asc),
            )
            .await
            .map_err(read::error)?;
        let mut turns = Vec::with_capacity(rows.rows().len());
        for row in rows.rows() {
            turns.push(self.turned(row).await?);
        }
        Ok(turns)
    }

    pub async fn latest(&self, strand: &str) -> Result<Option<turn::Turn>, String> {
        let strand = read::one(&self.core, "Strand", "tag", strand)
            .await?
            .ok_or_else(|| "strand not found".to_string())?;
        let row = self
            .core
            .one(
                &form("Turn")
                    .when("strand", Op::Eq, &strand.key().to_string())
                    .order("created", Rank::Desc)
                    .order("tag", Rank::Desc)
                    .top(1),
            )
            .await
            .map_err(read::error)?;
        match row {
            Some(row) => self.turned(&row).await.map(Some),
            None => Ok(None),
        }
    }

    pub async fn running(&self) -> Result<usize, String> {
        let turns = self.core.live("Turn").await.map_err(read::error)?;
        let complete = self
            .core
            .live("TurnCompletion")
            .await
            .map_err(read::error)?;
        let failed = self.core.live("TurnFailure").await.map_err(read::error)?;
        Ok(turns
            .iter()
            .filter(|turn| {
                !complete
                    .iter()
                    .any(|outcome| outcome.int("turn") == Some(turn.key()))
                    && !failed
                        .iter()
                        .any(|outcome| outcome.int("turn") == Some(turn.key()))
            })
            .count())
    }

    pub async fn complete_turn(
        &self,
        tag: &str,
        to: i64,
        finished: &str,
    ) -> Result<turn::Turn, String> {
        self.core
            .batch(async |tx| terminal::complete(tx, tag, to, finished).await)
            .await
            .map_err(read::error)?;
        self.turn(tag)
            .await?
            .ok_or_else(|| "finished turn missing".to_string())
    }

    pub async fn fail_turn(
        &self,
        tag: &str,
        error: &str,
        finished: &str,
    ) -> Result<turn::Turn, String> {
        self.core
            .batch(async |tx| terminal::fail(tx, tag, error, None, finished).await)
            .await
            .map_err(read::error)?;
        self.turn(tag)
            .await?
            .ok_or_else(|| "finished turn missing".to_string())
    }

    pub async fn fail_linked(
        &self,
        tag: &str,
        error: &str,
        incident: &str,
        finished: &str,
    ) -> Result<turn::Turn, String> {
        self.core
            .batch(async |tx| terminal::fail(tx, tag, error, Some(incident), finished).await)
            .await
            .map_err(read::error)?;
        self.turn(tag)
            .await?
            .ok_or_else(|| "linked failed turn missing".to_string())
    }

    async fn turned(&self, row: &keel::Row) -> Result<turn::Turn, String> {
        let key = row.key().to_string();
        let complete = read::one(&self.core, "TurnCompletion", "turn", &key).await?;
        let failed = read::one(&self.core, "TurnFailure", "turn", &key).await?;
        let (status, to, error, finished) = match (complete, failed) {
            (None, None) => (turn::Status::Running, None, None, None),
            (Some(done), None) => (
                turn::Status::Completed,
                Some(read::int(&done, "to")?),
                None,
                Some(read::text(&done, "finished")?.to_string()),
            ),
            (None, Some(failed)) => (
                turn::Status::Failed,
                None,
                Some(read::text(&failed, "error")?.to_string()),
                Some(read::text(&failed, "finished")?.to_string()),
            ),
            (Some(_), Some(_)) => return Err("turn has conflicting outcomes".to_string()),
        };
        Ok(turn::Turn {
            id: read::text(row, "tag")?.to_string(),
            strand: read::related(&self.core, "Strand", read::int(row, "strand")?).await?,
            trigger: decode_trigger(read::text(row, "trigger")?)?,
            source: row.text("source").map(str::to_string),
            from: read::int(row, "from")?,
            to,
            status,
            error,
            created: read::text(row, "created")?.to_string(),
            updated: read::text(row, "updated")?.to_string(),
            finished,
        })
    }
}

fn trigger(trigger: &turn::Trigger) -> &'static str {
    match trigger {
        turn::Trigger::StrandSend => "strand_send",
        turn::Trigger::System => "system",
    }
}

fn decode_trigger(value: &str) -> Result<turn::Trigger, String> {
    match value {
        "strand_send" => Ok(turn::Trigger::StrandSend),
        "system" => Ok(turn::Trigger::System),
        value => Err(format!("unknown turn trigger {value}")),
    }
}
