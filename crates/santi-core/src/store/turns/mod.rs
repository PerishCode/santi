use rusqlite::params;

use super::{
    Begun, Store, Stumble,
    db::{Database, drain},
};
use crate::{now, tag, turn::Turn};

mod completion;

pub use completion::Completion;

const BREADTH: usize = 4096;
mod fail;
mod stop;
use crate::{thinking, tool};
use fail::indict;

impl Store {
    pub fn tried(
        &self,
        strand: &str,
        trigger: &str,
        source: Option<&str>,
    ) -> Result<Option<Begun>, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let running: Option<i64> = tx
            .query_row(
                "SELECT 1 FROM turns WHERE strand_id = ?1 AND status = 'running' LIMIT 1",
                params![strand],
                |row| row.get(0),
            )
            .ok();
        if running.is_some() {
            return Ok(None);
        }
        let turn = tag("turn");
        let drained = drain(&tx, strand, &turn)?;
        if drained.messages.is_empty() {
            return Ok(None);
        }
        let now = now();
        tx.execute(
            r#"
            INSERT INTO turns (
              id, strand_id, trigger_type, trigger_ref,
              base_strand_seq, end_strand_seq, status, error_text,
              created_at, updated_at, finished_at
            )
            SELECT ?1, id, ?3, ?4, next_seq - 1, NULL, 'running', NULL, ?5, ?5, NULL
            FROM strands WHERE id = ?2
            "#,
            params![turn, strand, trigger, source, now],
        )
        .map_err(|error| error.to_string())?;
        Database::new(&tx).begin(strand, &turn, &drained.inboxes, None)?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(Some(Begun {
            turn: Database::new(&conn)
                .turn(&turn)?
                .ok_or_else(|| "created turn missing".to_string())?,
            drained: drained.messages,
        }))
    }

    pub fn latest(&self, strand: &str) -> Result<Option<Turn>, String> {
        let conn = self.conn.lock().unwrap();
        let id: Option<String> = conn
            .query_row(
                "SELECT id FROM turns WHERE strand_id = ?1 ORDER BY created_at DESC, id DESC LIMIT 1",
                params![strand],
                |row| row.get(0),
            )
            .ok();
        match id {
            Some(id) => Database::new(&conn).turn(&id),
            None => Ok(None),
        }
    }

    pub fn reconciled(&self) -> Result<usize, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let now = now();
        let mut stmt = tx
            .prepare(
                r#"
                SELECT t.id, t.strand_id, s.cause
                FROM turns t
                LEFT JOIN turn_stops s ON s.turn_id = t.id
                WHERE t.status = 'running'
                "#,
            )
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        drop(stmt);
        for (turn, strand, stopped) in &rows {
            let cause = stopped.as_deref().map(crate::turn::Cause::decode);
            let (before, during, detail) = match cause {
                Some(cause) => (
                    "turn_stopped_before_dispatch",
                    "turn_stopped_during_dispatch",
                    format!("interrupted by {}", cause.encode()),
                ),
                None => (
                    "restart_before_dispatch",
                    "restart_during_dispatch",
                    "interrupted by restart".to_string(),
                ),
            };
            Database::new(&tx).reconcile(turn, before, during, &now)?;
            tx.execute(
                r#"
                UPDATE turns
                SET status = 'failed', error_text = ?2,
                    updated_at = ?3, finished_at = ?3
                WHERE id = ?1 AND status = 'running'
                "#,
                params![turn, detail, now],
            )
            .map_err(|error| error.to_string())?;
            match cause {
                Some(cause) => {
                    stop::mark(
                        &tx,
                        stop::Mark {
                            strand,
                            turn,
                            cause,
                            now: &now,
                        },
                    )?;
                    tx.execute(
                        "UPDATE turn_stops SET settled_at = COALESCE(settled_at, ?2) WHERE turn_id = ?1",
                        params![turn, now],
                    )
                    .map_err(|error| error.to_string())?;
                    Database::new(&tx).fail(turn, None, &now)?;
                }
                None => {
                    let error = indict(
                        &tx,
                        strand,
                        turn,
                        Stumble {
                            operation: "turn.restart_reconcile",
                            detail: "interrupted by restart",
                        },
                    )?;
                    Database::new(&tx).fail(turn, error.incident.as_deref(), &now)?;
                }
            }
        }
        tx.commit().map_err(|error| error.to_string())?;
        Ok(rows.len())
    }

    pub fn running(&self) -> Result<usize, String> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM turns WHERE status = 'running'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count as usize)
        .map_err(|error| error.to_string())
    }

    pub fn awaiting(&self) -> Result<Vec<String>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT DISTINCT strand_id FROM strand_inbox")
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map([], |row| row.get(0))
            .map_err(|error| error.to_string())?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|error| error.to_string())?);
        }
        Ok(out)
    }

    pub fn called(&self, turn: &str) -> Result<Vec<tool::Call>, String> {
        let conn = self.conn.lock().unwrap();
        Database::new(&conn).called(turn)
    }

    pub fn thought(&self, turn: &str) -> Result<Vec<thinking::Span>, String> {
        let conn = self.conn.lock().unwrap();
        Database::new(&conn).thought(turn)
    }

    pub fn replied(&self, turn: &str) -> Result<Vec<tool::Reply>, String> {
        let conn = self.conn.lock().unwrap();
        Database::new(&conn).replied(turn)
    }
}
