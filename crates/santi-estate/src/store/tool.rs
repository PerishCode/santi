use super::{Store, read, write};
use keel::{Op, Rank, form};
use santi_model::tool;

pub struct CallDraft<'a> {
    pub tag: &'a str,
    pub turn: &'a str,
    pub tool: &'a str,
    pub arguments: &'a serde_json::Value,
    pub created: &'a str,
}

#[derive(Clone, Copy)]
pub struct ReplyDraft<'a> {
    pub tag: &'a str,
    pub call: &'a str,
    pub output: Option<&'a serde_json::Value>,
    pub error: Option<&'a str>,
    pub created: &'a str,
}

impl Store {
    pub async fn create_call(&self, draft: CallDraft<'_>) -> Result<tool::Call, String> {
        let tag = draft.tag.to_string();
        let arguments =
            serde_json::to_string(draft.arguments).map_err(|error| error.to_string())?;
        self.core
            .batch(async |tx| put_call(tx, draft, &arguments).await)
            .await
            .map_err(read::error)?;
        self.call(&tag)
            .await?
            .ok_or_else(|| "created tool call missing".to_string())
    }

    pub async fn call(&self, tag: &str) -> Result<Option<tool::Call>, String> {
        let Some(row) = read::one(&self.core, "ToolCall", "tag", tag).await? else {
            return Ok(None);
        };
        self.decode_call(&row).await.map(Some)
    }

    pub async fn calls(&self, strand: &str) -> Result<Vec<tool::Call>, String> {
        let strand = read::one(&self.core, "Strand", "tag", strand)
            .await?
            .ok_or_else(|| "strand not found".to_string())?;
        let entries = self
            .core
            .ask(
                &form("StrandEntry")
                    .when("strand", Op::Eq, &strand.key().to_string())
                    .when("target_type", Op::Eq, "tool_call")
                    .order("sequence", Rank::Asc),
            )
            .await
            .map_err(read::error)?;
        let mut calls = Vec::with_capacity(entries.rows().len());
        for entry in entries.rows() {
            let tag = read::text(entry, "target")?;
            let row = read::one(&self.core, "ToolCall", "tag", tag)
                .await?
                .ok_or_else(|| format!("tool call {tag} missing"))?;
            calls.push(self.decode_call(&row).await?);
        }
        Ok(calls)
    }

    pub async fn called(&self, turn: &str) -> Result<Vec<tool::Call>, String> {
        let turn = read::one(&self.core, "Turn", "tag", turn)
            .await?
            .ok_or_else(|| "turn not found".to_string())?;
        let rows = self
            .core
            .ask(
                &form("ToolCall")
                    .when("turn", Op::Eq, &turn.key().to_string())
                    .order("created", Rank::Asc)
                    .order("tag", Rank::Asc),
            )
            .await
            .map_err(read::error)?;
        let mut calls = Vec::with_capacity(rows.rows().len());
        for row in rows.rows() {
            calls.push(self.decode_call(row).await?);
        }
        Ok(calls)
    }

    pub async fn create_reply(&self, draft: ReplyDraft<'_>) -> Result<tool::Reply, String> {
        let output = draft
            .output
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| error.to_string())?;
        self.core
            .batch(async |tx| put_reply(tx, draft, output.as_deref()).await)
            .await
            .map_err(read::error)?;
        self.reply(draft.tag)
            .await?
            .ok_or_else(|| "created tool reply missing".to_string())
    }

    pub async fn reply(&self, tag: &str) -> Result<Option<tool::Reply>, String> {
        let Some(row) = read::one(&self.core, "ToolResult", "tag", tag).await? else {
            return Ok(None);
        };
        self.decode_reply(&row).await.map(Some)
    }

    pub async fn results(&self, strand: &str) -> Result<Vec<tool::Reply>, String> {
        let strand = read::one(&self.core, "Strand", "tag", strand)
            .await?
            .ok_or_else(|| "strand not found".to_string())?;
        let entries = self
            .core
            .ask(
                &form("StrandEntry")
                    .when("strand", Op::Eq, &strand.key().to_string())
                    .when("target_type", Op::Eq, "tool_result")
                    .order("sequence", Rank::Asc),
            )
            .await
            .map_err(read::error)?;
        let mut replies = Vec::with_capacity(entries.rows().len());
        for entry in entries.rows() {
            let tag = read::text(entry, "target")?;
            let row = read::one(&self.core, "ToolResult", "tag", tag)
                .await?
                .ok_or_else(|| format!("tool result {tag} missing"))?;
            replies.push(self.decode_reply(&row).await?);
        }
        Ok(replies)
    }

    pub async fn replied(&self, turn: &str) -> Result<Vec<tool::Reply>, String> {
        let calls = self.called(turn).await?;
        let mut replies = Vec::new();
        for call in calls {
            let Some(row) = read::one(&self.core, "ToolCall", "tag", &call.id).await? else {
                return Err(format!("tool call {} missing", call.id));
            };
            let rows = self
                .core
                .ask(
                    &form("ToolResult")
                        .when("call", Op::Eq, &row.key().to_string())
                        .order("created", Rank::Asc)
                        .order("tag", Rank::Asc),
                )
                .await
                .map_err(read::error)?;
            for row in rows.rows() {
                replies.push(self.decode_reply(row).await?);
            }
        }
        replies.sort_by(|left, right| {
            left.created
                .cmp(&right.created)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(replies)
    }

    async fn decode_call(&self, row: &keel::Row) -> Result<tool::Call, String> {
        Ok(tool::Call {
            id: read::text(row, "tag")?.to_string(),
            turn: read::related(&self.core, "Turn", read::int(row, "turn")?).await?,
            tool: read::text(row, "tool")?.to_string(),
            arguments: serde_json::from_str(read::text(row, "arguments")?)
                .map_err(|error| error.to_string())?,
            created: read::text(row, "created")?.to_string(),
        })
    }

    async fn decode_reply(&self, row: &keel::Row) -> Result<tool::Reply, String> {
        let key = row.key().to_string();
        let output = read::one(&self.core, "ToolOutput", "result", &key).await?;
        let failure = read::one(&self.core, "ToolFailure", "result", &key).await?;
        let (output, error) = match (output, failure) {
            (Some(output), None) => (
                Some(
                    serde_json::from_str(read::text(&output, "output")?)
                        .map_err(|error| error.to_string())?,
                ),
                None,
            ),
            (None, Some(failure)) => (None, Some(read::text(&failure, "error")?.to_string())),
            (None, None) => return Err("tool result has no outcome".to_string()),
            (Some(_), Some(_)) => return Err("tool result has conflicting outcomes".to_string()),
        };
        Ok(tool::Reply {
            id: read::text(row, "tag")?.to_string(),
            call: read::related(&self.core, "ToolCall", read::int(row, "call")?).await?,
            output,
            error,
            created: read::text(row, "created")?.to_string(),
        })
    }
}

pub(in crate::store) async fn put_call(
    tx: &mut keel::Tx<'_, keel::adapt::db::Sqlite>,
    draft: CallDraft<'_>,
    arguments: &str,
) -> Result<(), keel::adapt::Error> {
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
    tx.put(
        "ToolCall",
        &[
            ("tag", draft.tag),
            ("tool", draft.tool),
            ("arguments", arguments),
            ("created", draft.created),
            ("turn", &turn.key().to_string()),
        ],
    )
    .await?;
    write::append(tx, &strand, "tool_call", draft.tag, draft.created).await?;
    Ok(())
}

pub(in crate::store) async fn put_reply(
    tx: &mut keel::Tx<'_, keel::adapt::db::Sqlite>,
    draft: ReplyDraft<'_>,
    output: Option<&str>,
) -> Result<(), keel::adapt::Error> {
    if output.is_some() == draft.error.is_some() {
        return Err(keel::adapt::Error::Adapt(
            "tool reply needs exactly one outcome".into(),
        ));
    }
    let call = tx
        .one(&form("ToolCall").when("tag", Op::Eq, draft.call))
        .await?
        .ok_or_else(|| keel::adapt::Error::Missing(draft.call.into()))?;
    let turn = call
        .int("turn")
        .ok_or_else(|| keel::adapt::Error::Adapt("call turn missing".into()))?;
    let turn = tx
        .one(&form("Turn").when("id", Op::Eq, &turn.to_string()))
        .await?
        .ok_or_else(|| keel::adapt::Error::Missing("call turn".into()))?;
    let strand = turn
        .int("strand")
        .ok_or_else(|| keel::adapt::Error::Adapt("turn strand missing".into()))?;
    let strand = tx
        .one(&form("Strand").when("id", Op::Eq, &strand.to_string()))
        .await?
        .ok_or_else(|| keel::adapt::Error::Missing("turn strand".into()))?;
    let result = tx
        .put(
            "ToolResult",
            &[
                ("tag", draft.tag),
                ("created", draft.created),
                ("call", &call.key().to_string()),
            ],
        )
        .await?;
    match (output, draft.error) {
        (Some(output), None) => {
            tx.put(
                "ToolOutput",
                &[("output", output), ("result", &result.to_string())],
            )
            .await?;
        }
        (None, Some(error)) => {
            tx.put(
                "ToolFailure",
                &[("error", error), ("result", &result.to_string())],
            )
            .await?;
        }
        _ => unreachable!("tool reply outcome was validated"),
    }
    write::append(tx, &strand, "tool_result", draft.tag, draft.created).await?;
    Ok(())
}
