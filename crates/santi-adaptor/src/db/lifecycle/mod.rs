use std::path::Path;

use rusqlite::Connection;

use crate::{VERSION, schema::SCHEMA};

mod expand;

use expand::expand;

pub fn version(path: impl AsRef<Path>) -> Result<Option<u32>, String> {
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

    if version == VERSION {
        let tx = conn
            .unchecked_transaction()
            .map_err(|error| error.to_string())?;
        expand(&tx)?;
        tx.execute_batch(SCHEMA)
            .map_err(|error| error.to_string())?;
        return tx.commit().map_err(|error| error.to_string());
    }

    if version == 0 && empty(conn)? {
        let tx = conn
            .unchecked_transaction()
            .map_err(|error| error.to_string())?;
        tx.execute_batch(SCHEMA)
            .map_err(|error| error.to_string())?;
        tx.pragma_update(None, "user_version", VERSION)
            .map_err(|error| error.to_string())?;
        return tx.commit().map_err(|error| error.to_string());
    }

    if (32..=38).contains(&version) {
        let tx = conn
            .unchecked_transaction()
            .map_err(|error| error.to_string())?;
        if version <= 34 {
            reshape(&tx)?;
        }
        expand(&tx)?;
        tx.execute_batch(SCHEMA)
            .map_err(|error| error.to_string())?;
        tx.pragma_update(None, "user_version", VERSION)
            .map_err(|error| error.to_string())?;
        return tx.commit().map_err(|error| error.to_string());
    }

    Err(format!(
        "unsupported schema version {version}; expected 32 through {VERSION}; database left unchanged"
    ))
}

fn empty(conn: &Connection) -> Result<bool, String> {
    conn.query_row(
        "SELECT COUNT(*) = 0 FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
        [],
        |row| row.get(0),
    )
    .map_err(|error| error.to_string())
}

fn reshape(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        DROP TABLE IF EXISTS reply_outbox;
        DROP TABLE IF EXISTS window_messages;
        "#,
    )
    .map_err(|error| error.to_string())?;

    if table(conn, "strand_effects")? {
        conn.execute_batch(
            r#"
            ALTER TABLE strand_effects RENAME TO strand_effects_v34;
            CREATE TABLE strand_effects (
                id TEXT PRIMARY KEY,
                strand_id TEXT NOT NULL,
                turn_id TEXT NOT NULL,
                tool_call_id TEXT UNIQUE,
                effect_type TEXT NOT NULL,
                state TEXT NOT NULL CHECK (state IN (
                    'prepared', 'dispatching', 'unknown',
                    'settled_applied', 'settled_not_applied'
                )),
                result_ref TEXT,
                error_text TEXT,
                metadata TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                dispatched_at TEXT,
                settled_at TEXT
            );
            INSERT INTO strand_effects (
                id, strand_id, turn_id, tool_call_id, effect_type, state,
                result_ref, error_text, metadata, created_at, updated_at,
                dispatched_at, settled_at
            )
            SELECT
                id, strand_id, turn_id, tool_call_id, effect_type,
                CASE state
                    WHEN 'confirmed' THEN 'settled_applied'
                    WHEN 'not_dispatched' THEN 'settled_not_applied'
                    WHEN 'resolved_applied' THEN 'settled_applied'
                    WHEN 'resolved_not_applied' THEN 'settled_not_applied'
                    ELSE state
                END,
                result_ref, error_text, metadata, created_at, updated_at,
                dispatched_at, settled_at
            FROM strand_effects_v34;
            DROP TABLE strand_effects_v34;
            "#,
        )
        .map_err(|error| error.to_string())?;
    }

    if table(conn, "thinking_spans")? {
        conn.execute_batch(
            r#"
            ALTER TABLE thinking_spans RENAME TO thinking_spans_v34;
            CREATE TABLE thinking_spans (
                id TEXT PRIMARY KEY,
                turn_id TEXT NOT NULL,
                provider_response_id TEXT,
                state TEXT NOT NULL CHECK (state IN ('running', 'completed', 'failed')),
                summary TEXT,
                completion_reason TEXT CHECK (
                    completion_reason IS NULL OR
                    completion_reason IN ('spoke', 'called', 'finished')
                ),
                error_text TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                finished_at TEXT,
                CHECK (
                    (state = 'failed' AND error_text IS NOT NULL) OR
                    (state <> 'failed' AND error_text IS NULL)
                )
            );
            INSERT INTO thinking_spans (
                id, turn_id, provider_response_id, state, summary,
                completion_reason, error_text, created_at, updated_at, finished_at
            )
            SELECT
                id, turn_id, provider_response_id, state, summary,
                CASE completion_reason
                    WHEN 'first_text_delta' THEN 'spoke'
                    WHEN 'tool_call_requested' THEN 'called'
                    WHEN 'provider_completed' THEN 'finished'
                    ELSE completion_reason
                END,
                error_text, created_at, updated_at, finished_at
            FROM thinking_spans_v34;
            DROP TABLE thinking_spans_v34;
            "#,
        )
        .map_err(|error| error.to_string())?;
    }

    if table(conn, "inbox_receipts")? {
        conn.execute_batch(
            r#"
            ALTER TABLE inbox_receipts RENAME TO inbox_receipts_v34;
            CREATE TABLE inbox_receipts (
                id TEXT PRIMARY KEY,
                strand_id TEXT NOT NULL,
                state TEXT NOT NULL CHECK (state IN (
                    'accepted', 'recovered', 'driving', 'failed', 'completed'
                )),
                accepted_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            INSERT INTO inbox_receipts (
                id, strand_id, state, accepted_at, updated_at
            )
            SELECT
                id, strand_id,
                CASE state
                    WHEN 'mechanically_recovered' THEN 'recovered'
                    WHEN 'turn_failed' THEN 'failed'
                    ELSE state
                END,
                accepted_at, updated_at
            FROM inbox_receipts_v34;
            DROP TABLE inbox_receipts_v34;
            "#,
        )
        .map_err(|error| error.to_string())?;
    }

    if table(conn, "receipt_transitions")? {
        conn.execute_batch(
            r#"
            ALTER TABLE receipt_transitions RENAME TO receipt_transitions_v34;
            CREATE TABLE receipt_transitions (
                id TEXT PRIMARY KEY,
                inbox_id TEXT NOT NULL,
                sequence INTEGER NOT NULL CHECK (sequence > 0),
                state TEXT NOT NULL CHECK (state IN (
                    'accepted', 'recovered', 'driving', 'failed', 'completed'
                )),
                turn_id TEXT,
                incident_id TEXT,
                reconstructed_from TEXT,
                occurred_at TEXT NOT NULL,
                UNIQUE (inbox_id, sequence)
            );
            INSERT INTO receipt_transitions (
                id, inbox_id, sequence, state, turn_id, incident_id,
                reconstructed_from, occurred_at
            )
            SELECT
                id, inbox_id, sequence,
                CASE state
                    WHEN 'mechanically_recovered' THEN 'recovered'
                    WHEN 'turn_failed' THEN 'failed'
                    ELSE state
                END,
                turn_id, incident_id, reconstructed_from, occurred_at
            FROM receipt_transitions_v34;
            DROP TABLE receipt_transitions_v34;
            "#,
        )
        .map_err(|error| error.to_string())?;
    }

    conn.execute_batch("DROP TABLE IF EXISTS effect_transitions;")
        .map_err(|error| error.to_string())
}

fn table(conn: &Connection, name: &str) -> Result<bool, String> {
    conn.query_row(
        "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [name],
        |row| row.get(0),
    )
    .map_err(|error| error.to_string())
}
