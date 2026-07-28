use super::{Store, read, write};
use keel::{Op, Rank, form};
use santi_model::{message, strand};

mod compact;
mod fork;
mod projection;

pub use compact::CompactDraft;
pub use fork::ForkDraft;

pub struct MessageDraft<'a> {
    pub tag: &'a str,
    pub strand: &'a str,
    pub actor: message::Role,
    pub actor_id: &'a str,
    pub kind: message::Kind,
    pub content: &'a message::Content,
    pub state: message::State,
    pub request: bool,
    pub created: &'a str,
}

impl Store {
    pub async fn place(&self, draft: MessageDraft<'_>) -> Result<message::Placed, String> {
        let content = serde_json::to_string(draft.content).map_err(|error| error.to_string())?;
        let tag = draft.tag.to_string();
        self.core
            .batch(async |tx| {
                let strand = tx
                    .one(&form("Strand").when("tag", Op::Eq, draft.strand))
                    .await?
                    .ok_or_else(|| keel::adapt::Error::Missing(draft.strand.to_string()))?;
                tx.put(
                    "Message",
                    &[
                        ("tag", draft.tag),
                        ("actor_type", role(&draft.actor)),
                        ("actor", draft.actor_id),
                        ("kind", kind(&draft.kind)),
                        ("content", &content),
                        ("state", state(&draft.state)),
                        ("request", if draft.request { "true" } else { "false" }),
                        ("created", draft.created),
                        ("updated", draft.created),
                    ],
                )
                .await?;
                write::append(tx, &strand, "message", draft.tag, draft.created).await?;
                Ok(())
            })
            .await
            .map_err(read::error)?;
        self.message(&tag)
            .await?
            .ok_or_else(|| "created message missing".to_string())
    }

    pub async fn message(&self, tag: &str) -> Result<Option<message::Placed>, String> {
        let Some(message) = read::one(&self.core, "Message", "tag", tag).await? else {
            return Ok(None);
        };
        let Some(entry) = self
            .core
            .one(
                &form("StrandEntry")
                    .when("target_type", Op::Eq, "message")
                    .when("target", Op::Eq, tag),
            )
            .await
            .map_err(read::error)?
        else {
            return Err("message entry missing".to_string());
        };
        self.placed(&entry, &message).await.map(Some)
    }

    pub async fn messages(&self, strand: &str) -> Result<Vec<message::Placed>, String> {
        let strand = read::one(&self.core, "Strand", "tag", strand)
            .await?
            .ok_or_else(|| "strand not found".to_string())?;
        let entries = self
            .core
            .ask(
                &form("StrandEntry")
                    .when("strand", Op::Eq, &strand.key().to_string())
                    .when("target_type", Op::Eq, "message")
                    .order("sequence", Rank::Asc),
            )
            .await
            .map_err(read::error)?;
        let mut placed = Vec::with_capacity(entries.rows().len());
        for entry in entries.rows() {
            let tag = read::text(entry, "target")?;
            let Some(message) = read::one(&self.core, "Message", "tag", tag).await? else {
                return Err(format!("message {tag} missing"));
            };
            if message.text("deleted").is_none() {
                placed.push(self.placed(entry, &message).await?);
            }
        }
        Ok(placed)
    }

    async fn placed(&self, entry: &keel::Row, row: &keel::Row) -> Result<message::Placed, String> {
        let content: message::Content =
            serde_json::from_str(read::text(row, "content")?).map_err(|error| error.to_string())?;
        let message = message::Message {
            id: read::text(row, "tag")?.to_string(),
            role: decode_role(read::text(row, "actor_type")?)?,
            actor: read::text(row, "actor")?.to_string(),
            kind: decode_kind(read::text(row, "kind")?)?,
            state: decode_state(read::text(row, "state")?)?,
            version: read::int(row, "version")?,
            deleted: row.text("deleted").map(str::to_string),
            created: read::text(row, "created")?.to_string(),
            updated: read::text(row, "updated")?.to_string(),
            content,
        };
        let relation = message::Relation {
            strand: read::related(&self.core, "Strand", read::int(entry, "strand")?).await?,
            message: message.id.clone(),
            seq: read::int(entry, "sequence")?,
            created: read::text(entry, "created")?.to_string(),
        };
        let text = message.content.rendered();
        Ok(message::Placed {
            relation,
            message,
            text,
        })
    }
}

fn role(role: &message::Role) -> &'static str {
    match role {
        message::Role::Soul => "soul",
        message::Role::System => "system",
    }
}

fn kind(kind: &message::Kind) -> &'static str {
    match kind {
        message::Kind::Text => "text",
        message::Kind::SantiSystem => "santi_system",
    }
}

fn state(state: &message::State) -> &'static str {
    match state {
        message::State::Pending => "pending",
        message::State::Fixed => "fixed",
        message::State::Aborted => "aborted",
    }
}

fn decode_role(value: &str) -> Result<message::Role, String> {
    match value {
        "soul" => Ok(message::Role::Soul),
        "system" => Ok(message::Role::System),
        value => Err(format!("unknown message role {value}")),
    }
}

fn decode_kind(value: &str) -> Result<message::Kind, String> {
    match value {
        "text" => Ok(message::Kind::Text),
        "santi_system" => Ok(message::Kind::SantiSystem),
        value => Err(format!("unknown message kind {value}")),
    }
}

fn decode_state(value: &str) -> Result<message::State, String> {
    match value {
        "pending" => Ok(message::State::Pending),
        "fixed" => Ok(message::State::Fixed),
        "aborted" => Ok(message::State::Aborted),
        value => Err(format!("unknown message state {value}")),
    }
}

fn decode_target(value: &str) -> Result<strand::Target, String> {
    match value {
        "message" => Ok(strand::Target::Message),
        "thinking" => Ok(strand::Target::Thinking),
        "tool_call" => Ok(strand::Target::ToolCall),
        "tool_result" => Ok(strand::Target::ToolResult),
        value => Err(format!("unknown strand target {value}")),
    }
}
