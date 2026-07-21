use rusqlite::{Connection, OptionalExtension, params};

use santi_model::{MessageContent, WindowAuthor, WindowTranscriptEntry};

pub struct Reserved {
    pub message_id: String,
    pub inbox_id: String,
    pub content_hash: String,
    pub cursor: Option<i64>,
    pub received_at: String,
}

pub fn window_message(
    conn: &Connection,
    participant_id: &str,
    client_message_id: &str,
) -> Result<Option<Reserved>, String> {
    conn.query_row(
        r#"
        SELECT message_id, inbox_id, content_hash, cursor, received_at
        FROM window_messages
        WHERE participant_id = ?1 AND client_message_id = ?2
        "#,
        params![participant_id, client_message_id],
        |row| {
            Ok(Reserved {
                message_id: row.get(0)?,
                inbox_id: row.get(1)?,
                content_hash: row.get(2)?,
                cursor: row.get(3)?,
                received_at: row.get(4)?,
            })
        },
    )
    .optional()
    .map_err(|error| error.to_string())
}

pub fn labeled_strand_id(
    conn: &Connection,
    soul_id: &str,
    label: &str,
) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT id FROM strands WHERE soul_id = ?1 AND external_label = ?2",
        params![soul_id, label],
        |row| row.get(0),
    )
    .optional()
    .map_err(|error| error.to_string())
}

pub fn window_transcript(
    conn: &Connection,
    strand_id: &str,
    since: i64,
    limit: usize,
) -> Result<(Vec<WindowTranscriptEntry>, bool, bool), String> {
    let mut stmt = conn
        .prepare(
            r#"
            SELECT e.strand_seq, m.id, m.actor_type, m.content, m.created_at, w.received_at
            FROM r_strand_entries e
            JOIN messages m ON m.id = e.target_id
            LEFT JOIN window_messages w ON w.message_id = m.id
            WHERE e.strand_id = ?1
              AND e.target_type = 'message'
              AND e.strand_seq > ?2
              AND m.message_kind = 'text'
              AND m.deleted_at IS NULL
            ORDER BY e.strand_seq ASC
            LIMIT ?3
            "#,
        )
        .map_err(|error| error.to_string())?;
    let fetch = limit + 1;
    let mut entries = stmt
        .query_map(params![strand_id, since, fetch as i64], |row| {
            let seq: i64 = row.get(0)?;
            let message_id: String = row.get(1)?;
            let actor: String = row.get(2)?;
            let content_json: String = row.get(3)?;
            let created_at: String = row.get(4)?;
            let received_at: Option<String> = row.get(5)?;
            Ok((
                seq,
                message_id,
                actor,
                content_json,
                created_at,
                received_at,
            ))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(
            |(seq, message_id, actor, content_json, created_at, received_at)| {
                let text = serde_json::from_str::<MessageContent>(&content_json)
                    .map(|content| content.content_text())
                    .unwrap_or(content_json);
                let author = if actor == "soul" {
                    WindowAuthor::Assistant
                } else {
                    WindowAuthor::Human
                };
                let at = match author {
                    WindowAuthor::Human => received_at.unwrap_or(created_at),
                    WindowAuthor::Assistant => created_at,
                };
                WindowTranscriptEntry {
                    message_id,
                    seq,
                    author,
                    text,
                    at,
                }
            },
        )
        .collect::<Vec<_>>();
    let has_more = entries.len() > limit;
    entries.truncate(limit);
    drop(stmt);
    let empty = if since == 0 && entries.is_empty() {
        true
    } else if entries.is_empty() {
        let count: i64 = conn
            .query_row(
                r#"
                SELECT COUNT(*)
                FROM r_strand_entries e
                JOIN messages m ON m.id = e.target_id
                WHERE e.strand_id = ?1 AND e.target_type = 'message'
                  AND m.message_kind = 'text' AND m.deleted_at IS NULL
                "#,
                params![strand_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        count == 0
    } else {
        false
    };
    Ok((entries, has_more, empty))
}
