use std::path::Path;

use rusqlite::Connection;

use crate::{SCHEMA_VERSION, schema::SCHEMA};

pub fn read_schema_version(path: impl AsRef<Path>) -> Result<Option<u32>, String> {
    if !path.as_ref().exists() {
        return Ok(None);
    }
    let conn = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .map_err(|error| error.to_string())?;
    conn.query_row("PRAGMA user_version", [], |row| row.get::<_, u32>(0))
        .map(Some)
        .map_err(|error| error.to_string())
}

pub fn migrate(conn: &Connection) -> Result<(), String> {
    let version = conn
        .query_row("PRAGMA user_version", [], |row| row.get::<_, u32>(0))
        .map_err(|error| error.to_string())?;
    if version != SCHEMA_VERSION {
        conn.execute_batch(
            r#"
                DROP TABLE IF EXISTS provider_replay_material;
                DROP TABLE IF EXISTS response_stream_deltas;
                DROP TABLE IF EXISTS response_runs;
                DROP TABLE IF EXISTS message_text_contents;
                DROP TABLE IF EXISTS conversations;
                DROP TABLE IF EXISTS r_strand_entries;
                DROP TABLE IF EXISTS strand_inbox;
                DROP TABLE IF EXISTS receipt_transitions;
                DROP TABLE IF EXISTS inbox_receipts;
                DROP TABLE IF EXISTS effect_transitions;
                DROP TABLE IF EXISTS downstream_ingest;
                DROP TABLE IF EXISTS downstreams;
                DROP TABLE IF EXISTS turn_outbox;
                DROP TABLE IF EXISTS reply_outbox;
                DROP TABLE IF EXISTS window_messages;
                DROP TABLE IF EXISTS compacts;
                DROP TABLE IF EXISTS error_transitions;
                DROP TABLE IF EXISTS error_incidents;
                DROP TABLE IF EXISTS strand_blocks;
                DROP TABLE IF EXISTS rejected_deliveries;
                DROP TABLE IF EXISTS thinking_spans;
                DROP TABLE IF EXISTS tool_results;
                DROP TABLE IF EXISTS tool_calls;
                DROP TABLE IF EXISTS turns;
                DROP TABLE IF EXISTS strands;
                DROP TABLE IF EXISTS strand_effects;
                DROP TABLE IF EXISTS message_events;
                DROP TABLE IF EXISTS messages;
                DROP TABLE IF EXISTS webhooks;
                DROP TABLE IF EXISTS soul_profiles;
                DROP TABLE IF EXISTS souls;
                -- Historical (pre-strand-rename) table names, so a clean wipe
                -- reaches a `session`-era DB (e.g. the live box before this
                -- migration). Harmless no-ops on a fresh DB.
                DROP TABLE IF EXISTS soul_sessions;
                DROP TABLE IF EXISTS sessions;
                DROP TABLE IF EXISTS session_profiles;
                DROP TABLE IF EXISTS r_session_messages;
                DROP TABLE IF EXISTS session_effects;
                DROP TABLE IF EXISTS accounts;
                "#,
        )
        .map_err(|error| error.to_string())?;
    }
    conn.execute_batch(SCHEMA)
        .map_err(|error| error.to_string())?;
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)
        .map_err(|error| error.to_string())?;
    Ok(())
}
