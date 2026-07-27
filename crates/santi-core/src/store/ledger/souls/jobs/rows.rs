use rusqlite::{OptionalExtension, Row};

use super::Record;
use crate::job;

pub(super) const COLUMNS: &str = r#"
    id, soul_id, strand_id, turn_id, tool_call_id, effect_id,
    description, command, cwd, timeout_seconds, output_limit_bytes,
    request_sha256, capability_sha256, generation, supervisor_ref,
    state, reason, exit_code, created_at, updated_at, accepted_at,
    started_at, finished_at, acknowledged_at
"#;

pub(super) fn record(conn: &rusqlite::Connection, id: &str) -> Result<Option<Record>, String> {
    let sql = format!("SELECT {COLUMNS} FROM jobs WHERE id = ?1");
    conn.query_row(&sql, [id], decode)
        .optional()
        .map_err(|error| error.to_string())
}

pub(super) fn decode(row: &Row<'_>) -> rusqlite::Result<Record> {
    let state = job::State::decode(&row.get::<_, String>(15)?).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            15,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, error)),
        )
    })?;
    Ok(Record {
        job: job::Job {
            id: row.get(0)?,
            origin: job::Origin {
                soul: row.get(1)?,
                strand: row.get(2)?,
                turn: row.get(3)?,
                call: row.get(4)?,
                effect: row.get(5)?,
            },
            description: row.get(6)?,
            command: row.get(7)?,
            cwd: row.get(8)?,
            timeout_seconds: unsigned(row, 9)?,
            output_limit_bytes: unsigned(row, 10)?,
            state,
            reason: row.get(16)?,
            exit_code: row.get(17)?,
            created: row.get(18)?,
            updated: row.get(19)?,
            accepted: row.get(20)?,
            started: row.get(21)?,
            finished: row.get(22)?,
            acknowledged: row.get(23)?,
        },
        generation: row.get(13)?,
        supervisor: row.get(14)?,
    })
}

fn unsigned(row: &Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value = row.get::<_, i64>(index)?;
    u64::try_from(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })
}
