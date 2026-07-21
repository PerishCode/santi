use rusqlite::{Connection, params};
use serde_json::json;

use santi_model::{prefixed_id, timestamp_now};

struct LegacyEffect {
    id: String,
    strand_id: String,
    effect_type: String,
    idempotency_key: String,
    status: String,
    source_hook_id: String,
    source_turn_id: String,
    result_ref: Option<String>,
    error_text: Option<String>,
    created_at: String,
    updated_at: String,
}

pub fn migrate_v25_to_v26(conn: &Connection) -> Result<(), String> {
    if !table_exists(conn, "strand_effects")? {
        return Ok(());
    }
    if column_exists(conn, "strand_effects", "turn_id")?
        && column_exists(conn, "strand_effects", "state")?
    {
        return Ok(());
    }

    let tx = conn
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    let mut stmt = tx
        .prepare(
            r#"
            SELECT id, strand_id, effect_type, idempotency_key, status,
                   source_hook_id, source_turn_id, result_ref, error_text,
                   created_at, updated_at
            FROM strand_effects
            ORDER BY created_at, id
            "#,
        )
        .map_err(|error| error.to_string())?;
    let legacy = stmt
        .query_map([], |row| {
            Ok(LegacyEffect {
                id: row.get(0)?,
                strand_id: row.get(1)?,
                effect_type: row.get(2)?,
                idempotency_key: row.get(3)?,
                status: row.get(4)?,
                source_hook_id: row.get(5)?,
                source_turn_id: row.get(6)?,
                result_ref: row.get(7)?,
                error_text: row.get(8)?,
                created_at: row.get(9)?,
                updated_at: row.get(10)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    drop(stmt);

    tx.execute_batch(
        r#"
        ALTER TABLE strand_effects RENAME TO strand_effects_v25;
        DROP INDEX IF EXISTS idx_strand_effects_strand_created_at;
        DROP INDEX IF EXISTS idx_strand_effects_lookup;

        CREATE TABLE strand_effects (
            id TEXT PRIMARY KEY,
            strand_id TEXT NOT NULL,
            turn_id TEXT NOT NULL,
            tool_call_id TEXT UNIQUE,
            effect_type TEXT NOT NULL,
            state TEXT NOT NULL CHECK (state IN (
                'prepared', 'dispatching', 'unknown', 'confirmed', 'not_dispatched',
                'resolved_applied', 'resolved_not_applied'
            )),
            result_ref TEXT,
            error_text TEXT,
            metadata TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            dispatched_at TEXT,
            settled_at TEXT
        );

        CREATE TABLE effect_transitions (
            id TEXT PRIMARY KEY,
            effect_id TEXT NOT NULL,
            sequence INTEGER NOT NULL CHECK (sequence > 0),
            state TEXT NOT NULL CHECK (state IN (
                'prepared', 'dispatching', 'unknown', 'confirmed', 'not_dispatched',
                'resolved_applied', 'resolved_not_applied'
            )),
            reason TEXT NOT NULL,
            evidence TEXT,
            occurred_at TEXT NOT NULL,
            UNIQUE (effect_id, sequence)
        );

        CREATE INDEX idx_strand_effects_strand_created_at
        ON strand_effects (strand_id, created_at);
        CREATE INDEX idx_strand_effects_turn_created_at
        ON strand_effects (turn_id, created_at);
        CREATE INDEX idx_strand_effects_state_updated_at
        ON strand_effects (state, updated_at);
        CREATE INDEX idx_effect_transitions_effect_sequence
        ON effect_transitions (effect_id, sequence);
        "#,
    )
    .map_err(|error| error.to_string())?;

    for effect in legacy {
        let metadata = serde_json::to_string(&json!({
            "legacy_v25": {
                "idempotency_key": effect.idempotency_key,
                "status": effect.status,
                "source_hook_id": effect.source_hook_id,
            }
        }))
        .map_err(|error| error.to_string())?;
        tx.execute(
            r#"
            INSERT INTO strand_effects (
              id, strand_id, turn_id, tool_call_id, effect_type, state,
              result_ref, error_text, metadata, created_at, updated_at,
              dispatched_at, settled_at
            )
            VALUES (?1, ?2, ?3, NULL, ?4, 'unknown', ?5, ?6, ?7, ?8, ?9, NULL, NULL)
            "#,
            params![
                effect.id,
                effect.strand_id,
                effect.source_turn_id,
                effect.effect_type,
                effect.result_ref,
                effect.error_text,
                metadata.as_str(),
                effect.created_at,
                effect.updated_at,
            ],
        )
        .map_err(|error| error.to_string())?;
        tx.execute(
            r#"
            INSERT INTO effect_transitions (
              id, effect_id, sequence, state, reason, evidence, occurred_at
            ) VALUES (?1, ?2, 1, 'unknown', 'legacy_import', ?3, ?4)
            "#,
            params![
                prefixed_id("efx"),
                effect.id,
                metadata.as_str(),
                timestamp_now(),
            ],
        )
        .map_err(|error| error.to_string())?;
    }
    tx.execute("DROP TABLE strand_effects_v25", [])
        .map_err(|error| error.to_string())?;
    tx.commit().map_err(|error| error.to_string())
}

fn table_exists(conn: &Connection, table: &str) -> Result<bool, String> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            params![table],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    Ok(count > 0)
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool, String> {
    let count: i64 = conn
        .query_row(
            &format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name = ?1"),
            params![column],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    Ok(count > 0)
}
