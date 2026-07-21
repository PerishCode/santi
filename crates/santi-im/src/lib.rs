use std::sync::{Arc, Mutex};

use rusqlite::{Connection, OptionalExtension, params};

use santi_model::{ImDelivery, ImDeliveryMode, ImInboxEntry, prefixed_id, timestamp_now};

const IM_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS im_participants (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('human', 'soul')),
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS im_inbox (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    participant_id TEXT NOT NULL,
    from_ref TEXT,
    turn_id TEXT,
    message_id TEXT,
    delivery_mode TEXT CHECK (
        delivery_mode IS NULL OR delivery_mode IN ('explicit', 'automatic')
    ),
    content TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_im_inbox_participant_seq ON im_inbox (participant_id, seq);
CREATE UNIQUE INDEX IF NOT EXISTS idx_im_inbox_turn
ON im_inbox(turn_id)
WHERE turn_id IS NOT NULL;
"#;

pub struct Reply<'a> {
    pub strand: &'a str,
    pub turn: &'a str,
    pub participant: &'a str,
    pub message: Option<&'a str>,
    pub content: &'a str,
    pub mode: ImDeliveryMode,
}

#[derive(Clone)]
pub struct ImStore {
    conn: Arc<Mutex<Connection>>,
}

impl ImStore {
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|error| error.to_string())?;
        conn.execute_batch(IM_SCHEMA)
            .map_err(|error| error.to_string())?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn ensure_participant(&self, id: &str, kind: &str) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            r#"
            INSERT INTO im_participants (id, kind, created_at)
            VALUES (?1, ?2, ?3)
            ON CONFLICT(id) DO NOTHING
            "#,
            params![id, kind, timestamp_now()],
        )
        .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn enqueue_inbox(
        &self,
        participant_id: &str,
        from_ref: Option<&str>,
        content: &str,
    ) -> Result<ImInboxEntry, String> {
        let conn = self.conn.lock().unwrap();
        let id = prefixed_id("imx");
        let now = timestamp_now();
        conn.execute(
            r#"
            INSERT INTO im_inbox (id, participant_id, from_ref, content, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
            params![id, participant_id, from_ref, content, now],
        )
        .map_err(|error| error.to_string())?;
        Ok(ImInboxEntry {
            seq: conn.last_insert_rowid(),
            id,
            participant_id: participant_id.to_string(),
            from_ref: from_ref.map(str::to_string),
            turn_id: None,
            message_id: None,
            delivery_mode: None,
            content: content.to_string(),
            created_at: now,
        })
    }

    pub fn poll_inbox(
        &self,
        participant_id: &str,
        since: i64,
    ) -> Result<Vec<ImInboxEntry>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                r#"
                SELECT seq, id, participant_id, from_ref, turn_id, message_id,
                       delivery_mode, content, created_at
                FROM im_inbox
                WHERE participant_id = ?1 AND seq > ?2
                ORDER BY seq ASC
                "#,
            )
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map(params![participant_id, since], map_inbox_entry)
            .map_err(|error| error.to_string())?;
        rows.map(|row| row.map_err(|error| error.to_string()))
            .collect()
    }

    pub fn enqueue_reply(&self, reply: Reply<'_>) -> Result<(ImInboxEntry, bool), String> {
        let conn = self.conn.lock().unwrap();
        let now = timestamp_now();
        conn.execute(
            r#"
            INSERT INTO im_participants (id, kind, created_at)
            VALUES (?1, 'human', ?2)
            ON CONFLICT(id) DO NOTHING
            "#,
            params![reply.participant, now],
        )
        .map_err(|error| error.to_string())?;
        let inserted = conn
            .execute(
                r#"
                INSERT OR IGNORE INTO im_inbox (
                  id, participant_id, from_ref, turn_id, message_id,
                  delivery_mode, content, created_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                "#,
                params![
                    prefixed_id("imx"),
                    reply.participant,
                    reply.strand,
                    reply.turn,
                    reply.message,
                    delivery_mode_db(&reply.mode),
                    reply.content,
                    now,
                ],
            )
            .map_err(|error| error.to_string())?
            == 1;
        let entry = reply_for_turn(&conn, reply.turn)?
            .ok_or_else(|| format!("IM reply for turn {} missing after insert", reply.turn))?;
        if entry.participant_id != reply.participant
            || entry.from_ref.as_deref() != Some(reply.strand)
        {
            return Err(format!(
                "IM reply idempotency collision for turn {}",
                reply.turn
            ));
        }
        Ok((entry, inserted))
    }

    pub fn deliver_reply(&self, event: &santi_protocol::ReplyEvent) -> Result<(), String> {
        self.enqueue_reply(Reply {
            strand: &event.strand_id,
            turn: &event.turn_id,
            participant: &event.participant_id,
            message: event.message_id.as_deref(),
            content: &event.content,
            mode: event.mode,
        })
        .map(|_| ())
    }

    pub fn deliveries_for_turns(&self, turn_ids: &[String]) -> Result<Vec<ImDelivery>, String> {
        if turn_ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn.lock().unwrap();
        let placeholders = turn_ids.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
        let mut stmt = conn
            .prepare(&format!(
                r#"
                SELECT DISTINCT seq, id, participant_id, from_ref, turn_id, message_id,
                       delivery_mode, created_at
                FROM im_inbox
                WHERE turn_id IN ({placeholders})
                ORDER BY seq
                "#
            ))
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(turn_ids), |row| {
                let mode = row.get::<_, String>(6)?;
                Ok(ImDelivery {
                    seq: row.get(0)?,
                    id: row.get(1)?,
                    participant_id: row.get(2)?,
                    strand_id: row.get(3)?,
                    turn_id: row.get(4)?,
                    message_id: row.get(5)?,
                    delivery_mode: delivery_mode_from_db(&mode),
                    created_at: row.get(7)?,
                })
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }
}

fn reply_for_turn(conn: &Connection, turn_id: &str) -> Result<Option<ImInboxEntry>, String> {
    conn.query_row(
        r#"
        SELECT seq, id, participant_id, from_ref, turn_id, message_id,
               delivery_mode, content, created_at
        FROM im_inbox WHERE turn_id = ?1
        "#,
        params![turn_id],
        map_inbox_entry,
    )
    .optional()
    .map_err(|error| error.to_string())
}

fn map_inbox_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<ImInboxEntry> {
    Ok(ImInboxEntry {
        seq: row.get(0)?,
        id: row.get(1)?,
        participant_id: row.get(2)?,
        from_ref: row.get(3)?,
        turn_id: row.get(4)?,
        message_id: row.get(5)?,
        delivery_mode: row
            .get::<_, Option<String>>(6)?
            .map(|mode| delivery_mode_from_db(&mode)),
        content: row.get(7)?,
        created_at: row.get(8)?,
    })
}

fn delivery_mode_db(mode: &ImDeliveryMode) -> &'static str {
    match mode {
        ImDeliveryMode::Explicit => "explicit",
        ImDeliveryMode::Automatic => "automatic",
    }
}

fn delivery_mode_from_db(mode: &str) -> ImDeliveryMode {
    match mode {
        "explicit" => ImDeliveryMode::Explicit,
        _ => ImDeliveryMode::Automatic,
    }
}
