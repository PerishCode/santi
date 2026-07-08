use rusqlite::params;

use super::{
    SantiStore,
    db::{compacts_for_strand, message_seq_in_strand, strand_by_id},
};
use crate::{Strand, prefixed_id, timestamp_now};

impl SantiStore {
    pub fn fork_strand(&self, parent_strand_id: &str, fork_point: i64) -> Result<Strand, String> {
        if fork_point < 0 {
            return Err("fork_point must be >= 0".to_string());
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let parent = strand_by_id(&tx, parent_strand_id)?
            .ok_or_else(|| "parent strand not found".to_string())?;
        let parent_last_seq = parent.next_seq - 1;
        if fork_point > parent_last_seq {
            return Err(format!(
                "fork_point {fork_point} is past parent end {parent_last_seq}"
            ));
        }

        let child_id = prefixed_id("ss");
        let now = timestamp_now();
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
                parent.soul_id,
                fork_point + 1,
                parent.last_seen_strand_seq.min(fork_point),
                parent.id,
                fork_point,
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
                .query_map(params![parent_strand_id, fork_point], |row| {
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
        for (target_type, target_id, strand_seq, created_at) in entries {
            tx.execute(
                r#"
                INSERT INTO r_strand_entries (
                  strand_id, target_type, target_id, strand_seq, created_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5)
                "#,
                params![child_id, target_type, target_id, strand_seq, created_at],
            )
            .map_err(|error| error.to_string())?;
        }

        for compact in compacts_for_strand(&tx, parent_strand_id)? {
            let Some(start_seq) =
                message_seq_in_strand(&tx, parent_strand_id, &compact.start_message_id)?
            else {
                continue;
            };
            let Some(end_seq) =
                message_seq_in_strand(&tx, parent_strand_id, &compact.end_message_id)?
            else {
                continue;
            };
            if start_seq <= fork_point && end_seq <= fork_point {
                tx.execute(
                    r#"
                    INSERT INTO compacts (id, strand_id, summary, start_message_id, end_message_id)
                    VALUES (?1, ?2, ?3, ?4, ?5)
                    "#,
                    params![
                        prefixed_id("cmp"),
                        child_id,
                        compact.summary,
                        compact.start_message_id,
                        compact.end_message_id,
                    ],
                )
                .map_err(|error| error.to_string())?;
            }
        }

        tx.commit().map_err(|error| error.to_string())?;
        strand_by_id(&conn, &child_id)?.ok_or_else(|| "forked strand missing".to_string())
    }

    pub(crate) fn delete_fork_child_strand(&self, child_strand_id: &str) -> Result<(), String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let child = strand_by_id(&tx, child_strand_id)?
            .ok_or_else(|| "fork child not found".to_string())?;
        if child.parent_strand_id.is_none() {
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
