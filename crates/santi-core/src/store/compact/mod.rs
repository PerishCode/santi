use rusqlite::params;
use serde_json::Value;

use crate::{now, tag};

use super::{Store, db::Database, span::Span};

mod plan;
use crate::compact;
use plan::*;

struct Plan {
    span: Span,
    absorbed: Vec<String>,
    collapsed: i64,
}

pub(crate) struct Collapse<'a> {
    pub strand: &'a str,
    pub from: &'a str,
    pub to: &'a str,
    pub summary: &'a str,
    pub metadata: Option<Value>,
}

impl Store {
    pub fn compact(
        &self,
        strand: &str,
        first: &str,
        last: &str,
        summary: &str,
    ) -> Result<compact::Report, String> {
        self.noted(Collapse {
            strand,
            from: first,
            to: last,
            summary,
            metadata: None,
        })
    }

    pub(crate) fn noted(&self, collapse: Collapse<'_>) -> Result<compact::Report, String> {
        let Collapse {
            strand,
            from,
            to,
            summary,
            metadata,
        } = collapse;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let plan = planned(&tx, strand, from, to)?;

        for id in &plan.absorbed {
            tx.execute("DELETE FROM compacts WHERE id = ?1", params![id])
                .map_err(|error| error.to_string())?;
        }
        let compact = tag("cmp");
        let now = now();
        let blob = metadata
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| error.to_string())?;
        tx.execute(
            r#"
            INSERT INTO compacts (
              id, strand_id, summary, start_message_id, end_message_id, created_at, metadata
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
            params![compact, strand, summary, from, to, now, blob],
        )
        .map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())?;

        Ok(compact::Report {
            compact,
            first: from.to_string(),
            last: to.to_string(),
            from: plan.span.from,
            to: plan.span.to,
            absorbed: plan.absorbed,
            collapsed: plan.collapsed,
            dry: false,
            active_incident_resolved: false,
            before: None,
            after: None,
            ratio: None,
        })
    }

    pub(crate) fn previewing(
        &self,
        strand: &str,
        first: &str,
        last: &str,
    ) -> Result<compact::Report, String> {
        let conn = self.conn.lock().unwrap();
        let plan = planned(&conn, strand, first, last)?;
        Ok(compact::Report {
            compact: tag("cmp_preview"),
            first: first.to_string(),
            last: last.to_string(),
            from: plan.span.from,
            to: plan.span.to,
            absorbed: plan.absorbed,
            collapsed: plan.collapsed,
            dry: true,
            active_incident_resolved: false,
            before: None,
            after: None,
            ratio: None,
        })
    }

    pub(crate) fn seated(&self, strand: &str, seq: i64) -> Result<Option<String>, String> {
        let conn = self.conn.lock().unwrap();
        seated(&conn, strand, seq)
    }

    pub(crate) fn annotate(&self, compact: &str, metadata: Value) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        let blob = serde_json::to_string(&metadata).map_err(|error| error.to_string())?;
        conn.execute(
            "UPDATE compacts SET metadata = ?2 WHERE id = ?1",
            params![compact, blob],
        )
        .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn page(
        &self,
        compact: &str,
        keyword: Option<&str>,
        page_index: i64,
        page_size: i64,
    ) -> Result<Option<compact::Page>, String> {
        let conn = self.conn.lock().unwrap();
        let database = Database::new(&conn);
        let Some(compact) = database.held(compact)? else {
            return Ok(None);
        };
        let mut entries = Vec::new();
        if let (Some(from), Some(to)) = (
            database.seat(&compact.strand, &compact.first)?,
            database.seat(&compact.strand, &compact.last)?,
        ) {
            let needle = keyword
                .map(str::trim)
                .filter(|k| !k.is_empty())
                .map(str::to_lowercase);
            let mut stmt = conn
                .prepare(
                    r#"
                    SELECT strand_seq, target_type, target_id
                    FROM r_strand_entries
                    WHERE strand_id = ?1 AND strand_seq BETWEEN ?2 AND ?3
                    ORDER BY strand_seq ASC
                    "#,
                )
                .map_err(|error| error.to_string())?;
            let rows = stmt
                .query_map(params![compact.strand, from, to], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })
                .map_err(|error| error.to_string())?;
            for row in rows {
                let (seq, kind, target) = row.map_err(|error| error.to_string())?;
                let text = entry(&conn, &kind, &target)?;
                if let Some(needle) = &needle
                    && !text.to_lowercase().contains(needle)
                {
                    continue;
                }
                entries.push(compact::Entry {
                    seq,
                    kind: targeted(&kind),
                    target,
                    text,
                });
            }
        }

        let total = entries.len() as i64;
        let skip = page_index.max(0).saturating_mul(page_size.max(0)).max(0) as usize;
        let take = page_size.max(0) as usize;
        let entries = entries.into_iter().skip(skip).take(take).collect();
        Ok(Some(compact::Page {
            compact: compact.id,
            first: compact.first,
            last: compact.last,
            total,
            page_index,
            page_size,
            entries,
        }))
    }
}
