use crate::store::{Store, read};
use keel::{Op, Rank, form};
use santi_model::compact;

mod page;
mod plan;

pub struct CompactDraft<'a> {
    pub tag: &'a str,
    pub strand: &'a str,
    pub first: &'a str,
    pub last: &'a str,
    pub summary: &'a str,
    pub metadata: Option<&'a serde_json::Value>,
    pub created: &'a str,
}

impl Store {
    pub async fn preview_compact(
        &self,
        tag: &str,
        strand: &str,
        first: &str,
        last: &str,
    ) -> Result<compact::Report, String> {
        let plan = self
            .core
            .batch(async |tx| plan::build(tx, strand, first, last).await)
            .await
            .map_err(read::error)?;
        Ok(plan.report(tag, true))
    }

    pub async fn create_compact(&self, draft: CompactDraft<'_>) -> Result<compact::Report, String> {
        if draft.summary.trim().is_empty() {
            return Err("compact summary must not be empty".to_string());
        }
        let metadata = draft
            .metadata
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| error.to_string())?;
        let plan = self
            .core
            .batch(async |tx| {
                let plan = plan::build(tx, draft.strand, draft.first, draft.last).await?;
                for (key, _) in &plan.absorbed {
                    tx.end("Compact", *key).await?;
                }
                let strand = plan.strand.to_string();
                let first = plan.first.to_string();
                let last = plan.last.to_string();
                let mut fields = vec![
                    ("tag", draft.tag),
                    ("summary", draft.summary),
                    ("created", draft.created),
                    ("strand", strand.as_str()),
                    ("first", first.as_str()),
                    ("last", last.as_str()),
                ];
                if let Some(metadata) = metadata.as_deref() {
                    fields.push(("metadata", metadata));
                }
                tx.put("Compact", &fields).await?;
                Ok(plan)
            })
            .await
            .map_err(read::error)?;
        self.compact(draft.tag)
            .await?
            .ok_or_else(|| "created compact missing".to_string())?;
        Ok(plan.report(draft.tag, false))
    }

    pub async fn compact(&self, tag: &str) -> Result<Option<compact::Compact>, String> {
        let Some(row) = read::one(&self.core, "Compact", "tag", tag).await? else {
            return Ok(None);
        };
        self.decode_compact(&row).await.map(Some)
    }

    pub async fn compacts(&self, strand: &str) -> Result<Vec<compact::Compact>, String> {
        let strand = read::one(&self.core, "Strand", "tag", strand)
            .await?
            .ok_or_else(|| "strand not found".to_string())?;
        let rows = self
            .core
            .ask(
                &form("Compact")
                    .when("strand", Op::Eq, &strand.key().to_string())
                    .order("created", Rank::Asc)
                    .order("tag", Rank::Asc),
            )
            .await
            .map_err(read::error)?;
        let mut held = Vec::with_capacity(rows.rows().len());
        for row in rows.rows() {
            held.push(self.decode_compact(row).await?);
        }
        Ok(held)
    }

    pub async fn seated(&self, strand: &str, sequence: i64) -> Result<Option<String>, String> {
        let Some(strand) = read::one(&self.core, "Strand", "tag", strand).await? else {
            return Ok(None);
        };
        let row = self
            .core
            .one(
                &form("StrandEntry")
                    .when("strand", Op::Eq, &strand.key().to_string())
                    .when("sequence", Op::Eq, &sequence.to_string())
                    .when("target_type", Op::Eq, "message"),
            )
            .await
            .map_err(read::error)?;
        Ok(row.and_then(|row| row.text("target").map(str::to_string)))
    }

    pub async fn compact_page(
        &self,
        tag: &str,
        keyword: Option<&str>,
        page_index: i64,
        page_size: i64,
    ) -> Result<Option<compact::Page>, String> {
        page::read(self, tag, keyword, page_index, page_size).await
    }

    pub async fn annotate_compact(
        &self,
        tag: &str,
        metadata: &serde_json::Value,
    ) -> Result<compact::Compact, String> {
        let metadata = serde_json::to_string(metadata).map_err(|error| error.to_string())?;
        self.core
            .batch(async |tx| {
                let row = tx
                    .one(&form("Compact").when("tag", Op::Eq, tag))
                    .await?
                    .ok_or_else(|| keel::adapt::Error::Missing(tag.into()))?;
                tx.set("Compact", row.key(), &[("metadata", metadata.as_str())])
                    .await
            })
            .await
            .map_err(read::error)?;
        self.compact(tag)
            .await?
            .ok_or_else(|| "annotated compact missing".to_string())
    }

    async fn decode_compact(&self, row: &keel::Row) -> Result<compact::Compact, String> {
        Ok(compact::Compact {
            id: read::text(row, "tag")?.to_string(),
            strand: read::related(&self.core, "Strand", read::int(row, "strand")?).await?,
            summary: read::text(row, "summary")?.to_string(),
            first: read::related(&self.core, "Message", read::int(row, "first")?).await?,
            last: read::related(&self.core, "Message", read::int(row, "last")?).await?,
            created: row.text("created").map(str::to_string),
            metadata: row
                .text("metadata")
                .map(serde_json::from_str)
                .transpose()
                .map_err(|error| error.to_string())?,
        })
    }
}
