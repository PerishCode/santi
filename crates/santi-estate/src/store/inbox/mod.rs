use super::{Store, read};
use keel::adapt::db::Sqlite;
use keel::{Core, Op, Rank, Row, form};
use santi_model::{ingest, message};
use std::collections::BTreeSet;

mod drain;
mod edge;
pub(in crate::store) mod receipt;
mod types;
pub(in crate::store) mod write;

pub use edge::{Accepted, DownstreamDraft, ReplayDraft, WebhookDraft};
pub use types::{Begun, DrainDraft, Inbox, InboxDraft, NoticeDraft, Offer, Opening};
pub(in crate::store) use write::offer as offer_in;

impl Store {
    pub async fn drain_turn(&self, draft: DrainDraft<'_>) -> Result<Opening, String> {
        drain::open(self, draft).await
    }

    pub async fn accept_inbox(&self, draft: InboxDraft<'_>, gate: usize) -> Result<Inbox, String> {
        let content = json(draft.content)?;
        let metadata = draft
            .source
            .and_then(|source| source.metadata.as_ref())
            .map(json)
            .transpose()?;
        self.core
            .batch(async |tx| write::accept(tx, &draft, gate, &content, metadata.as_deref()).await)
            .await
            .map_err(read::error)?;
        self.inbox(draft.tag)
            .await?
            .ok_or_else(|| "accepted inbox missing".to_string())
    }

    pub async fn offer_notice(&self, draft: NoticeDraft<'_>, gate: usize) -> Result<Offer, String> {
        self.core
            .batch(async |tx| write::offer(tx, draft, gate).await)
            .await
            .map_err(read::error)
    }

    pub async fn inbox(&self, tag: &str) -> Result<Option<Inbox>, String> {
        let Some(row) = read::one(&self.core, "StrandInbox", "tag", tag).await? else {
            return Ok(None);
        };
        decode(&self.core, &row).await.map(Some)
    }

    pub async fn inboxes(&self, strand: &str) -> Result<Vec<Inbox>, String> {
        let strand = read::one(&self.core, "Strand", "tag", strand)
            .await?
            .ok_or_else(|| "strand not found".to_string())?;
        let rows = self
            .core
            .ask(
                &form("StrandInbox")
                    .when("strand", Op::Eq, &strand.key().to_string())
                    .order("created", Rank::Asc)
                    .order("tag", Rank::Asc),
            )
            .await
            .map_err(read::error)?;
        let mut inboxes = Vec::with_capacity(rows.rows().len());
        for row in rows.rows() {
            inboxes.push(decode(&self.core, row).await?);
        }
        Ok(inboxes)
    }

    pub async fn receipt(
        &self,
        inbox: &str,
    ) -> Result<Option<santi_model::receipt::Status>, String> {
        let Some(mut status) = receipt::status(&self.core, inbox).await? else {
            return Ok(None);
        };
        status.effects = self.effects_for_receipt(inbox).await?;
        Ok(Some(status))
    }

    pub async fn advance_receipt(
        &self,
        inbox: &str,
        state: santi_model::receipt::State,
        turn: Option<&str>,
        incident: Option<&str>,
        rebuilt: Option<&str>,
        occurred: &str,
    ) -> Result<santi_model::receipt::Status, String> {
        receipt::advance(&self.core, inbox, state, turn, incident, rebuilt, occurred).await?;
        self.receipt(inbox)
            .await?
            .ok_or_else(|| "advanced receipt missing".to_string())
    }
}

async fn decode(core: &Core<Sqlite>, row: &Row) -> Result<Inbox, String> {
    let source = match row.text("source_type") {
        Some(kind) => Some(ingest::Source {
            kind: kind.to_string(),
            source: row.text("source_ref").map(str::to_string),
            metadata: row
                .text("source_metadata")
                .map(serde_json::from_str)
                .transpose()
                .map_err(|error| error.to_string())?,
        }),
        None => None,
    };
    let causes = row
        .text("coalesce_causes")
        .map(serde_json::from_str::<BTreeSet<String>>)
        .transpose()
        .map_err(|error| error.to_string())?
        .unwrap_or_default()
        .into_iter()
        .collect();
    Ok(Inbox {
        id: read::text(row, "tag")?.to_string(),
        strand: read::related(core, "Strand", read::int(row, "strand")?).await?,
        kind: decode_kind(read::text(row, "kind")?)?,
        content: serde_json::from_str(read::text(row, "content")?)
            .map_err(|error| error.to_string())?,
        source,
        coalesce_key: row.text("coalesce_key").map(str::to_string),
        coalesce_revision: row.int("coalesce_revision"),
        coalesce_causes: causes,
        created: read::text(row, "created")?.to_string(),
    })
}

fn kind(kind: &message::Kind) -> &'static str {
    match kind {
        message::Kind::Text => "text",
        message::Kind::SantiSystem => "santi_system",
    }
}

fn decode_kind(value: &str) -> Result<message::Kind, String> {
    match value {
        "text" => Ok(message::Kind::Text),
        "santi_system" => Ok(message::Kind::SantiSystem),
        value => Err(format!("unknown inbox kind {value}")),
    }
}

fn json(value: &impl serde::Serialize) -> Result<String, String> {
    serde_json::to_string(value).map_err(|error| error.to_string())
}
