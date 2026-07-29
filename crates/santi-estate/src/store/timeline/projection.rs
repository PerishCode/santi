use super::{Store, decode_role, read, target};
use keel::{Op, Rank, form};
use santi_model::{message, strand, stream};

impl Store {
    pub async fn entries(&self, strand: &str) -> Result<Vec<strand::Entry>, String> {
        let strand = read::one(&self.core, "Strand", "tag", strand)
            .await?
            .ok_or_else(|| "strand not found".to_string())?;
        let rows = self
            .core
            .ask(
                &form("StrandEntry")
                    .when("strand", Op::Eq, &strand.key().to_string())
                    .order("sequence", Rank::Asc),
            )
            .await
            .map_err(read::error)?;
        let mut entries = Vec::with_capacity(rows.rows().len());
        for row in rows.rows() {
            entries.push(strand::Entry {
                strand: read::related(&self.core, "Strand", read::int(row, "strand")?).await?,
                kind: target::decode(read::text(row, "target_type")?)?,
                target: read::text(row, "target")?.to_string(),
                seq: read::int(row, "sequence")?,
                created: read::text(row, "created")?.to_string(),
            });
        }
        Ok(entries)
    }

    pub async fn events(&self, strand: &str) -> Result<Vec<message::Event>, String> {
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
        let mut events = Vec::new();
        for entry in entries.rows() {
            let tag = read::text(entry, "target")?;
            let message = read::one(&self.core, "Message", "tag", tag)
                .await?
                .ok_or_else(|| format!("message {tag} missing"))?;
            let rows = self
                .core
                .ask(
                    &form("MessageEvent")
                        .when("message", Op::Eq, &message.key().to_string())
                        .order("created", Rank::Asc)
                        .order("tag", Rank::Asc),
                )
                .await
                .map_err(read::error)?;
            for row in rows.rows() {
                events.push(self.decode_event(row).await?);
            }
        }
        Ok(events)
    }

    pub async fn snapshot(&self, strand: &str) -> Result<Option<stream::Snapshot>, String> {
        let Some(strand) = self.strand(strand).await? else {
            return Ok(None);
        };
        Ok(Some(stream::Snapshot {
            messages: self.messages(&strand.id).await?,
            events: self.events(&strand.id).await?,
            turns: self.turns(&strand.id).await?,
            thinking: self.thinkings(&strand.id).await?,
            calls: self.calls(&strand.id).await?,
            results: self.results(&strand.id).await?,
            compacts: self.compacts(&strand.id).await?,
            effects: self.effects(&strand.id).await?,
            errors: self
                .incidents(&santi_error::Scope::new("strand", &strand.id), 100)
                .await?,
            strand,
        }))
    }

    async fn decode_event(&self, row: &keel::Row) -> Result<message::Event, String> {
        Ok(message::Event {
            id: read::text(row, "tag")?.to_string(),
            message: read::related(&self.core, "Message", read::int(row, "message")?).await?,
            action: read::text(row, "action")?.to_string(),
            role: decode_role(read::text(row, "actor_type")?)?,
            actor: read::text(row, "actor")?.to_string(),
            base_version: read::int(row, "base_version")?,
            payload: serde_json::from_str(read::text(row, "payload")?)
                .map_err(|error| error.to_string())?,
            created: read::text(row, "created")?.to_string(),
        })
    }
}
