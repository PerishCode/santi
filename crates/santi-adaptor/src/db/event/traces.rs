use rusqlite::params;

use super::Database;
use crate::rows::collected;
use santi_model::{tag, trace};

pub struct Recorded<'a> {
    pub boot: &'a str,
    pub span: i64,
    pub parent: Option<i64>,
    pub name: &'a str,
    pub tags: &'a str,
    pub opened: &'a str,
    pub closed: &'a str,
}

impl Database<'_> {
    pub fn recorded(&self, record: Recorded<'_>) -> Result<(), String> {
        self.conn
            .execute(
                r#"
        INSERT INTO trace_records (
          id, boot_id, span_id, parent_id, name, tags, opened_at, closed_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
                params![
                    tag("trc"),
                    record.boot,
                    record.span,
                    record.parent,
                    record.name,
                    record.tags,
                    record.opened,
                    record.closed,
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn traced(&self, key: &str, value: &str) -> Result<Vec<trace::Record>, String> {
        let mut stmt = self
            .conn
            .prepare(
                r#"
            SELECT name, tags, opened_at, closed_at
            FROM trace_records
            WHERE EXISTS (
              SELECT 1 FROM json_each(trace_records.tags)
              WHERE json_extract(json_each.value, '$.key') = ?1
                AND json_extract(json_each.value, '$.value') = ?2
            )
            ORDER BY opened_at, span_id
            "#,
            )
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map(params![key, value], |row| {
                Ok(trace::Record {
                    name: row.get(0)?,
                    tags: serde_json::from_str(&row.get::<_, String>(1)?).unwrap_or_default(),
                    opened: row.get(2)?,
                    closed: row.get(3)?,
                })
            })
            .map_err(|error| error.to_string())?;
        collected(rows)
    }
}
