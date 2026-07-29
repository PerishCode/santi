use super::{Store, read, write};
use keel::{Op, Rank, form};
use santi_model::thinking;

pub struct ThinkingDraft<'a> {
    pub tag: &'a str,
    pub turn: &'a str,
    pub response: Option<&'a str>,
    pub created: &'a str,
}

impl Store {
    pub async fn create_thinking(
        &self,
        draft: ThinkingDraft<'_>,
    ) -> Result<thinking::Span, String> {
        self.core
            .batch(async |tx| {
                let turn = tx
                    .one(&form("Turn").when("tag", Op::Eq, draft.turn))
                    .await?
                    .ok_or_else(|| keel::adapt::Error::Missing(draft.turn.into()))?;
                let strand = turn
                    .int("strand")
                    .ok_or_else(|| keel::adapt::Error::Adapt("turn strand missing".into()))?;
                let strand = tx
                    .one(&form("Strand").when("id", Op::Eq, &strand.to_string()))
                    .await?
                    .ok_or_else(|| keel::adapt::Error::Missing("turn strand".into()))?;
                let turn_key = turn.key().to_string();
                let mut fields = vec![
                    ("tag", draft.tag),
                    ("created", draft.created),
                    ("updated", draft.created),
                    ("turn", turn_key.as_str()),
                ];
                if let Some(response) = draft.response {
                    fields.push(("response", response));
                }
                tx.put("ThinkingSpan", &fields).await?;
                write::append(
                    tx,
                    write::Entry {
                        strand: &strand,
                        kind: "thinking",
                        target: draft.tag,
                        created: draft.created,
                    },
                )
                .await?;
                Ok(())
            })
            .await
            .map_err(read::error)?;
        self.thinking(draft.tag)
            .await?
            .ok_or_else(|| "created thinking span missing".to_string())
    }

    pub async fn update_thinking(
        &self,
        tag: &str,
        response: Option<&str>,
        summary: Option<&str>,
        updated: &str,
    ) -> Result<Option<thinking::Span>, String> {
        let mut fields = vec![("updated", updated)];
        if let Some(response) = response {
            fields.push(("response", response));
        }
        if let Some(summary) = summary {
            fields.push(("summary", summary));
        }
        let found = self
            .core
            .batch(async |tx| {
                let Some(row) = tx
                    .one(&form("ThinkingSpan").when("tag", Op::Eq, tag))
                    .await?
                else {
                    return Ok(false);
                };
                let key = row.key().to_string();
                let complete = tx
                    .one(&form("ThinkingCompletion").when("thinking", Op::Eq, &key))
                    .await?;
                let failed = tx
                    .one(&form("ThinkingFailure").when("thinking", Op::Eq, &key))
                    .await?;
                if complete.is_none() && failed.is_none() {
                    tx.set("ThinkingSpan", row.key(), &fields).await?;
                }
                Ok(true)
            })
            .await
            .map_err(read::error)?;
        if found {
            self.thinking(tag).await
        } else {
            Ok(None)
        }
    }

    pub async fn complete_thinking(
        &self,
        tag: &str,
        reason: thinking::Reason,
        finished: &str,
    ) -> Result<Option<thinking::Span>, String> {
        self.finish_thinking(tag, Some(reason), None, finished)
            .await
    }

    pub async fn fail_thinking(
        &self,
        tag: &str,
        error: &str,
        finished: &str,
    ) -> Result<Option<thinking::Span>, String> {
        self.finish_thinking(tag, None, Some(error), finished).await
    }

    pub async fn thinking(&self, tag: &str) -> Result<Option<thinking::Span>, String> {
        let Some(row) = read::one(&self.core, "ThinkingSpan", "tag", tag).await? else {
            return Ok(None);
        };
        self.decode_thinking(&row).await.map(Some)
    }

    pub async fn thinkings(&self, strand: &str) -> Result<Vec<thinking::Span>, String> {
        let strand = read::one(&self.core, "Strand", "tag", strand)
            .await?
            .ok_or_else(|| "strand not found".to_string())?;
        let entries = self
            .core
            .ask(
                &form("StrandEntry")
                    .when("strand", Op::Eq, &strand.key().to_string())
                    .when("target_type", Op::Eq, "thinking")
                    .order("sequence", Rank::Asc),
            )
            .await
            .map_err(read::error)?;
        let mut spans = Vec::with_capacity(entries.rows().len());
        for entry in entries.rows() {
            let tag = read::text(entry, "target")?;
            let row = read::one(&self.core, "ThinkingSpan", "tag", tag)
                .await?
                .ok_or_else(|| format!("thinking span {tag} missing"))?;
            spans.push(self.decode_thinking(&row).await?);
        }
        Ok(spans)
    }

    pub async fn thought(&self, turn: &str) -> Result<Vec<thinking::Span>, String> {
        let turn = read::one(&self.core, "Turn", "tag", turn)
            .await?
            .ok_or_else(|| "turn not found".to_string())?;
        let rows = self
            .core
            .ask(
                &form("ThinkingSpan")
                    .when("turn", Op::Eq, &turn.key().to_string())
                    .order("created", Rank::Asc)
                    .order("tag", Rank::Asc),
            )
            .await
            .map_err(read::error)?;
        let mut spans = Vec::with_capacity(rows.rows().len());
        for row in rows.rows() {
            spans.push(self.decode_thinking(row).await?);
        }
        Ok(spans)
    }

    async fn finish_thinking(
        &self,
        tag: &str,
        reason: Option<thinking::Reason>,
        error: Option<&str>,
        finished: &str,
    ) -> Result<Option<thinking::Span>, String> {
        let found = self
            .core
            .batch(async |tx| {
                let Some(row) = tx
                    .one(&form("ThinkingSpan").when("tag", Op::Eq, tag))
                    .await?
                else {
                    return Ok(false);
                };
                let key = row.key().to_string();
                let complete = tx
                    .one(&form("ThinkingCompletion").when("thinking", Op::Eq, &key))
                    .await?;
                let failed = tx
                    .one(&form("ThinkingFailure").when("thinking", Op::Eq, &key))
                    .await?;
                if complete.is_some() || failed.is_some() {
                    return Ok(true);
                }
                match (reason, error) {
                    (Some(reason), None) => {
                        tx.put(
                            "ThinkingCompletion",
                            &[
                                ("reason", reason_text(&reason)),
                                ("finished", finished),
                                ("thinking", &key),
                            ],
                        )
                        .await?;
                    }
                    (None, Some(error)) => {
                        tx.put(
                            "ThinkingFailure",
                            &[("error", error), ("finished", finished), ("thinking", &key)],
                        )
                        .await?;
                    }
                    _ => {
                        return Err(keel::adapt::Error::Adapt("invalid thinking outcome".into()));
                    }
                }
                tx.set("ThinkingSpan", row.key(), &[("updated", finished)])
                    .await?;
                Ok(true)
            })
            .await
            .map_err(read::error)?;
        if found {
            self.thinking(tag).await
        } else {
            Ok(None)
        }
    }

    async fn decode_thinking(&self, row: &keel::Row) -> Result<thinking::Span, String> {
        let key = row.key().to_string();
        let complete = read::one(&self.core, "ThinkingCompletion", "thinking", &key).await?;
        let failed = read::one(&self.core, "ThinkingFailure", "thinking", &key).await?;
        let (state, reason, error, finished) = match (complete, failed) {
            (None, None) => (thinking::State::Running, None, None, None),
            (Some(done), None) => (
                thinking::State::Completed,
                Some(decode_reason(read::text(&done, "reason")?)?),
                None,
                Some(read::text(&done, "finished")?.to_string()),
            ),
            (None, Some(failed)) => (
                thinking::State::Failed,
                None,
                Some(read::text(&failed, "error")?.to_string()),
                Some(read::text(&failed, "finished")?.to_string()),
            ),
            (Some(_), Some(_)) => return Err("thinking span has conflicting outcomes".to_string()),
        };
        Ok(thinking::Span {
            id: read::text(row, "tag")?.to_string(),
            turn: read::related(&self.core, "Turn", read::int(row, "turn")?).await?,
            response: row.text("response").map(str::to_string),
            state,
            summary: row.text("summary").map(str::to_string),
            completion_reason: reason,
            error,
            created: read::text(row, "created")?.to_string(),
            updated: read::text(row, "updated")?.to_string(),
            finished,
        })
    }
}

fn reason_text(reason: &thinking::Reason) -> &'static str {
    match reason {
        thinking::Reason::Spoke => "spoke",
        thinking::Reason::Called => "called",
        thinking::Reason::Finished => "finished",
    }
}

fn decode_reason(value: &str) -> Result<thinking::Reason, String> {
    match value {
        "spoke" => Ok(thinking::Reason::Spoke),
        "called" => Ok(thinking::Reason::Called),
        "finished" => Ok(thinking::Reason::Finished),
        value => Err(format!("unknown thinking reason {value}")),
    }
}
