use rusqlite::{Connection, params};
use serde_json::{Value, json};

use super::Database;
use crate::SYSTEM;
use santi_model::{message, strand};
use santi_model::{now, tag};

pub struct Drained {
    pub messages: Vec<message::Placed>,
    pub inboxes: Vec<String>,
}

pub fn drain(conn: &Connection, strand: &str, committing_turn_id: &str) -> Result<Drained, String> {
    let mut stmt = conn
        .prepare(
            r#"
            SELECT id, message_kind, content, source_type, source_ref, source_metadata,
                   coalesce_key, coalesce_causes, created_at
            FROM strand_inbox
            WHERE strand_id = ?1
            ORDER BY rowid ASC
            "#,
        )
        .map_err(|error| error.to_string())?;
    let pending = stmt
        .query_map(params![strand], |row| {
            Ok(Pending {
                id: row.get(0)?,
                kind: row.get(1)?,
                content: row.get(2)?,
                origin: row.get(3)?,
                source: row.get(4)?,
                metadata: row.get(5)?,
                key: row.get(6)?,
                causes: row.get(7)?,
                queued: row.get(8)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    drop(stmt);

    let now = now();
    let mut drained = Vec::with_capacity(pending.len());
    let mut inboxes = Vec::with_capacity(pending.len());
    let mut notices = Vec::new();
    for pending_entry in pending {
        if pending_entry.key.is_some() {
            notices.push(pending_entry);
            continue;
        }
        let message = insert(conn, &pending_entry.kind, &pending_entry.content, &now)?;
        let relation = Database::new(conn).entered(strand, strand::Target::Message, &message)?;
        record(
            conn,
            Entry {
                turn: committing_turn_id,
                now: &now,
                pending: &pending_entry,
                message: &message,
                sequence: relation.seq,
            },
        )?;
        inboxes.push(pending_entry.id.clone());
        drained.push(
            Database::new(conn)
                .message(&message)?
                .ok_or_else(|| "drained message missing".to_string())?,
        );
    }
    if !notices.is_empty() {
        let content = aggregate(&notices, &now)?;
        let message = insert(conn, "santi_system", &content, &now)?;
        let relation = Database::new(conn).entered(strand, strand::Target::Message, &message)?;
        for pending in notices {
            record(
                conn,
                Entry {
                    turn: committing_turn_id,
                    now: &now,
                    pending: &pending,
                    message: &message,
                    sequence: relation.seq,
                },
            )?;
            inboxes.push(pending.id);
        }
        drained.push(
            Database::new(conn)
                .message(&message)?
                .ok_or_else(|| "drained message missing".to_string())?,
        );
    }
    Ok(Drained {
        messages: drained,
        inboxes,
    })
}

fn insert(conn: &Connection, kind: &str, content: &str, now: &str) -> Result<String, String> {
    let message = tag("msg");
    conn.execute(
        r#"
            INSERT INTO messages (
              id, actor_type, actor_id, message_kind, content, state, version, is_request,
              deleted_at, created_at, updated_at
            )
            VALUES (?1, 'system', ?2, ?3, ?4, 'fixed', 1, 1, NULL, ?5, ?5)
            "#,
        params![message, SYSTEM, kind, content, now],
    )
    .map_err(|error| error.to_string())?;
    Ok(message)
}

struct Entry<'a> {
    turn: &'a str,
    now: &'a str,
    pending: &'a Pending,
    message: &'a str,
    sequence: i64,
}

fn record(conn: &Connection, entry: Entry<'_>) -> Result<(), String> {
    let database = Database::new(conn);
    database.drained(Drain {
        pending: entry.pending,
        message: entry.message,
        sequence: entry.sequence,
        turn: entry.turn,
        at: entry.now,
    })?;
    conn.execute(
        "UPDATE inbox_slots SET inbox_id = NULL, updated_at = ?2 WHERE inbox_id = ?1",
        params![entry.pending.id, entry.now],
    )
    .map_err(|error| error.to_string())?;
    conn.execute(
        "DELETE FROM strand_inbox WHERE id = ?1",
        params![entry.pending.id],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn aggregate(pending: &[Pending], now: &str) -> Result<String, String> {
    let mut lines = vec![
        "<system_message>".to_string(),
        "kind: inbox_attention".to_string(),
        "scope: strand_local".to_string(),
        "wake: true".to_string(),
        "obligation: false".to_string(),
        format!("assembled_at: {now}"),
        format!("items: {}", pending.len()),
    ];
    for item in pending {
        let content = serde_json::from_str::<message::Content>(&item.content)
            .map_err(|error| error.to_string())?;
        lines.push("---".to_string());
        if let Some(causes) = &item.causes {
            lines.push(format!("causes: {causes}"));
        }
        lines.push(content.rendered());
    }
    lines.push("</system_message>".to_string());
    serde_json::to_string(&message::Content::text(lines.join("\n")))
        .map_err(|error| error.to_string())
}

struct Pending {
    id: String,
    kind: String,
    content: String,
    origin: Option<String>,
    source: Option<String>,
    metadata: Option<String>,
    key: Option<String>,
    causes: Option<String>,
    queued: String,
}

struct Drain<'a> {
    pending: &'a Pending,
    message: &'a str,
    sequence: i64,
    turn: &'a str,
    at: &'a str,
}

impl Database<'_> {
    fn drained(&self, drain: Drain<'_>) -> Result<(), String> {
        let metadata = drain.pending.metadata.as_deref().map(|raw| {
            serde_json::from_str::<Value>(raw).unwrap_or_else(|_| json!({ "invalid_json": true }))
        });
        let payload = json!({
            "kind": "inbox_drain",
            "inbox": drain.pending.id.as_str(),
            "queued": drain.pending.queued.as_str(),
            "drained_at": drain.at,
            "committing_turn_id": drain.turn,
            "message": drain.message,
            "seq": drain.sequence,
            "source": {
                "type": drain.pending.origin.as_deref(),
                "ref": drain.pending.source.as_deref(),
                "metadata": metadata,
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
                    SYSTEM,
                    payload.to_string(),
                    drain.at
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }
}
