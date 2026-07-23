use rusqlite::params;

use super::{SantiStore, db::Database};
use crate::{now, strand::Strand, tag};

impl SantiStore {
    pub fn fork_strand(&self, parent: &str, fork: i64) -> Result<Strand, String> {
        if fork < 0 {
            return Err("fork must be >= 0".to_string());
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let database = Database::new(&tx);
        let parent = database
            .strand_by_id(parent)?
            .ok_or_else(|| "parent strand not found".to_string())?;
        let parent_last_seq = parent.next - 1;
        if fork > parent_last_seq {
            return Err(format!("fork {fork} is past parent end {parent_last_seq}"));
        }

        let child_id = tag("ss");
        let now = now();
        tx.execute(
            r#"
            INSERT INTO strands (
              id, soul_id, external_label, strand_memory, provider_state, next_seq,
              last_seen_strand_seq, parent_strand_id, fork_point, created_at, updated_at
            )
            VALUES (?1, ?2, NULL, '', NULL, ?3, ?4, ?5, ?6, ?7, ?7)
            "#,
            params![
                child_id,
                parent.soul,
                fork + 1,
                parent.seen.min(fork),
                parent.id,
                fork,
                now,
            ],
        )
        .map_err(|error| error.to_string())?;

        let mut entries = Vec::new();
        {
            let mut stmt = tx
                .prepare(
                    r#"
                    SELECT target_type, target_id, strand_seq, created_at
                    FROM r_strand_entries
                    WHERE strand_id = ?1 AND strand_seq <= ?2
                    ORDER BY strand_seq ASC
                    "#,
                )
                .map_err(|error| error.to_string())?;
            let rows = stmt
                .query_map(params![parent.id, fork], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })
                .map_err(|error| error.to_string())?;
            for row in rows {
                entries.push(row.map_err(|error| error.to_string())?);
            }
        }
        for (kind, target, seq, created) in entries {
            tx.execute(
                r#"
                INSERT INTO r_strand_entries (
                  strand_id, target_type, target_id, strand_seq, created_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5)
                "#,
                params![child_id, kind, target, seq, created],
            )
            .map_err(|error| error.to_string())?;
        }

        for compact in database.compacts_for_strand(&parent.id)? {
            let Some(start_seq) = database.message_seq_in_strand(&parent.id, &compact.first)?
            else {
                continue;
            };
            let Some(end_seq) = database.message_seq_in_strand(&parent.id, &compact.last)? else {
                continue;
            };
            if start_seq <= fork && end_seq <= fork {
                tx.execute(
                    r#"
                    INSERT INTO compacts (
                      id, strand_id, summary, start_message_id, end_message_id, created_at, metadata
                    )
                    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                    "#,
                    params![
                        tag("cmp"),
                        child_id,
                        compact.summary,
                        compact.first,
                        compact.last,
                        compact.created,
                        compact.metadata.map(|value| value.to_string()),
                    ],
                )
                .map_err(|error| error.to_string())?;
            }
        }

        tx.commit().map_err(|error| error.to_string())?;
        Database::new(&conn)
            .strand_by_id(&child_id)?
            .ok_or_else(|| "forked strand missing".to_string())
    }

    pub(crate) fn delete_fork_child_strand(&self, child_strand_id: &str) -> Result<(), String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let child = Database::new(&tx)
            .strand_by_id(child_strand_id)?
            .ok_or_else(|| "fork child not found".to_string())?;
        if child.parent.is_none() {
            return Err("refusing to delete a non-fork strand".to_string());
        }
        let turns: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM turns WHERE strand_id = ?1",
                params![child_strand_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if turns > 0 {
            return Err("refusing to delete a fork child that has turns".to_string());
        }
        tx.execute(
            "DELETE FROM compacts WHERE strand_id = ?1",
            params![child_strand_id],
        )
        .map_err(|error| error.to_string())?;
        tx.execute(
            "DELETE FROM r_strand_entries WHERE strand_id = ?1",
            params![child_strand_id],
        )
        .map_err(|error| error.to_string())?;
        tx.execute(
            "DELETE FROM strand_inbox WHERE strand_id = ?1",
            params![child_strand_id],
        )
        .map_err(|error| error.to_string())?;
        tx.execute(
            "DELETE FROM strands WHERE id = ?1",
            params![child_strand_id],
        )
        .map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(())
    }
}
