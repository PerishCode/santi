use rusqlite::{Connection, params};
use serde_json::{Value, json};

use super::Database;
use crate::SANTI_SYSTEM_ACTOR_ID;
use santi_model::{StrandMessage, StrandTargetType, now, tag};

pub struct DrainedInbox {
    pub messages: Vec<StrandMessage>,
    pub inbox_ids: Vec<String>,
}

pub fn drain_inbox_in_tx(
    conn: &Connection,
    strand: &str,
    committing_turn_id: &str,
) -> Result<DrainedInbox, String> {
    let mut stmt = conn
        .prepare(
            r#"
            SELECT id, message_kind, content, source_type, source_ref, source_metadata, created_at
            FROM strand_inbox
            WHERE strand_id = ?1
            ORDER BY rowid ASC
            "#,
        )
        .map_err(|error| error.to_string())?;
    let pending = stmt
        .query_map(params![strand], |row| {
            Ok(PendingInboxEntry {
                id: row.get(0)?,
                kind: row.get(1)?,
                content: row.get(2)?,
                origin: row.get(3)?,
                source: row.get(4)?,
                source_metadata: row.get(5)?,
                enqueued_at: row.get(6)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    drop(stmt);

    let now = now();
    let mut drained = Vec::with_capacity(pending.len());
    let mut inbox_ids = Vec::with_capacity(pending.len());
    for pending_entry in pending {
        let message = tag("msg");
        conn.execute(
            r#"
            INSERT INTO messages (
              id, actor_type, actor_id, message_kind, content, state, version, is_request,
              deleted_at, created_at, updated_at
            )
            VALUES (?1, 'system', ?2, ?3, ?4, 'fixed', 1, 1, NULL, ?5, ?5)
            "#,
            params![
                message,
                SANTI_SYSTEM_ACTOR_ID,
                pending_entry.kind.as_str(),
                pending_entry.content.as_str(),
                now
            ],
        )
        .map_err(|error| error.to_string())?;
        let database = Database::new(conn);
        let relation = database.append_entry_in_tx(strand, StrandTargetType::Message, &message)?;
        database.insert_drain(Drain {
            pending: &pending_entry,
            message: &message,
            sequence: relation.seq,
            turn: committing_turn_id,
            at: &now,
        })?;
        conn.execute(
            "DELETE FROM strand_inbox WHERE id = ?1",
            params![pending_entry.id],
        )
        .map_err(|error| error.to_string())?;
        inbox_ids.push(pending_entry.id.clone());
        drained.push(
            database
                .message_by_id(&message)?
                .ok_or_else(|| "drained message missing".to_string())?,
        );
    }
    Ok(DrainedInbox {
        messages: drained,
        inbox_ids,
    })
}

struct PendingInboxEntry {
    id: String,
    kind: String,
    content: String,
    origin: Option<String>,
    source: Option<String>,
    source_metadata: Option<String>,
    enqueued_at: String,
}

struct Drain<'a> {
    pending: &'a PendingInboxEntry,
    message: &'a str,
    sequence: i64,
    turn: &'a str,
    at: &'a str,
}

impl Database<'_> {
    fn insert_drain(&self, drain: Drain<'_>) -> Result<(), String> {
        let source_metadata = drain.pending.source_metadata.as_deref().map(|raw| {
            serde_json::from_str::<Value>(raw).unwrap_or_else(|_| json!({ "invalid_json": true }))
        });
        let payload = json!({
            "kind": "inbox_drain",
            "inbox": drain.pending.id.as_str(),
            "enqueued_at": drain.pending.enqueued_at.as_str(),
            "drained_at": drain.at,
            "committing_turn_id": drain.turn,
            "message": drain.message,
            "seq": drain.sequence,
            "source": {
                "type": drain.pending.origin.as_deref(),
                "ref": drain.pending.source.as_deref(),
                "metadata": source_metadata,
            }
        });
        self.conn
            .execute(
                r#"
                INSERT INTO message_events (
                  id, message_id, action, actor_type, actor_id, base_version, payload, created_at
                )
                VALUES (?1, ?2, 'insert', 'system', ?3, 1, ?4, ?5)
                "#,
                params![
                    tag("mev"),
                    drain.message,
                    SANTI_SYSTEM_ACTOR_ID,
                    payload.to_string(),
                    drain.at
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }
}
