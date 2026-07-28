use rusqlite::{OptionalExtension, params};

use crate::store::{Penned, Store, db::Database};
use crate::{message, now, strand, tag, turn};
use santi_adaptor::SYSTEM;

pub(crate) struct Stopped {
    pub(crate) turn: turn::Turn,
    pub(crate) marker: Option<Penned>,
}

impl Store {
    pub(crate) fn stopping(&self, id: &str) -> Result<Option<turn::Cause>, String> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT cause FROM turn_stops WHERE turn_id = ?1",
            [id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map(|cause| cause.map(|cause| turn::Cause::decode(&cause)))
        .map_err(|error| error.to_string())
    }

    pub(crate) fn request(
        &self,
        id: &str,
        cause: turn::Cause,
    ) -> Result<Option<turn::Stop>, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let Some(held) = Database::new(&tx).turn(id)? else {
            return Ok(None);
        };
        if held.status == turn::Status::Running {
            tx.execute(
                r#"
                INSERT OR IGNORE INTO turn_stops (
                  turn_id, cause, requested_at, settled_at
                ) VALUES (?1, ?2, ?3, NULL)
                "#,
                params![id, cause.encode(), now()],
            )
            .map_err(|error| error.to_string())?;
        }
        tx.commit().map_err(|error| error.to_string())?;
        projected(&conn, id).map(Some)
    }

    pub(crate) fn interrupt(&self, id: &str, cause: turn::Cause) -> Result<Stopped, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let now = now();
        tx.execute(
            r#"
            INSERT OR IGNORE INTO turn_stops (
              turn_id, cause, requested_at, settled_at
            )
            SELECT id, ?2, ?3, NULL FROM turns
            WHERE id = ?1 AND status = 'running'
            "#,
            params![id, cause.encode(), now],
        )
        .map_err(|error| error.to_string())?;
        let changed = tx
            .execute(
                r#"
                UPDATE turns
                SET status = 'failed', error_text = ?2,
                    updated_at = ?3, finished_at = ?3
                WHERE id = ?1 AND status = 'running'
                "#,
                params![id, format!("interrupted by {}", cause.encode()), now],
            )
            .map_err(|error| error.to_string())?;
        let marker = if changed == 1 {
            Database::new(&tx).reconcile(
                id,
                "turn_stopped_before_dispatch",
                "turn_stopped_during_dispatch",
                &now,
            )?;
            Database::new(&tx).fail(id, None, &now)?;
            let strand: String = tx
                .query_row("SELECT strand_id FROM turns WHERE id = ?1", [id], |row| {
                    row.get(0)
                })
                .map_err(|error| error.to_string())?;
            Some(mark(
                &tx,
                Mark {
                    strand: &strand,
                    turn: id,
                    cause,
                    now: &now,
                },
            )?)
        } else {
            None
        };
        tx.execute(
            "UPDATE turn_stops SET settled_at = COALESCE(settled_at, ?2) WHERE turn_id = ?1",
            params![id, now],
        )
        .map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())?;
        let turn = Database::new(&conn)
            .turn(id)?
            .ok_or_else(|| "turn not found".to_string())?;
        Ok(Stopped { turn, marker })
    }
}

fn projected(conn: &rusqlite::Connection, id: &str) -> Result<turn::Stop, String> {
    let turn = Database::new(conn)
        .turn(id)?
        .ok_or_else(|| "turn not found".to_string())?;
    let request = conn
        .query_row(
            "SELECT cause, requested_at, settled_at FROM turn_stops WHERE turn_id = ?1",
            [id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let (cause, requested, settled) = match request {
        Some((cause, requested, settled)) => {
            (Some(turn::Cause::decode(&cause)), Some(requested), settled)
        }
        None => (None, None, None),
    };
    Ok(turn::Stop {
        turn,
        accepted: cause.is_some(),
        cause,
        requested,
        settled,
    })
}

pub(super) struct Mark<'a> {
    pub(super) strand: &'a str,
    pub(super) turn: &'a str,
    pub(super) cause: turn::Cause,
    pub(super) now: &'a str,
}

pub(super) fn mark(conn: &rusqlite::Connection, mark: Mark<'_>) -> Result<Penned, String> {
    let Mark {
        strand,
        turn,
        cause,
        now,
    } = mark;
    let id = tag("msg");
    let content = serde_json::to_string(&message::Content::text(format!(
        "<system_message>\nThe previous turn ({turn}) was interrupted by {}. Its partial output and external effects may be incomplete; inspect durable effect state before retrying.\n</system_message>",
        cause.encode()
    )))
    .map_err(|error| error.to_string())?;
    conn.execute(
        r#"
        INSERT INTO messages (
          id, actor_type, actor_id, message_kind, content, state, version,
          is_request, deleted_at, created_at, updated_at
        )
        VALUES (?1, 'system', ?2, 'santi_system', ?3, 'fixed', 1, 0, NULL, ?4, ?4)
        "#,
        params![id, SYSTEM, content, now],
    )
    .map_err(|error| error.to_string())?;
    Database::new(conn).entered(strand, strand::Target::Message, &id)?;
    let message = Database::new(conn)
        .message(&id)?
        .ok_or_else(|| "interruption marker missing".to_string())?;
    Ok(Penned { message })
}
