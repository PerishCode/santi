use rusqlite::{OptionalExtension, params};

use crate::receipt;
use crate::store::SantiStore;
use crate::store::db::{Database, receipt_state_from_db};

impl SantiStore {
    pub fn receipt_status(&self, inbox: &str) -> Result<Option<receipt::Status>, String> {
        let conn = self.conn.lock().unwrap();
        let receipt = conn
            .query_row(
                r#"
                SELECT id, strand_id, state, accepted_at, updated_at
                FROM inbox_receipts WHERE id = ?1
                "#,
                params![inbox],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let Some((inbox, strand, state, accepted, updated)) = receipt else {
            return Ok(None);
        };
        let mut stmt = conn
            .prepare(
                r#"
                SELECT id, sequence, state, turn_id, incident_id,
                       reconstructed_from, occurred_at
                FROM receipt_transitions
                WHERE inbox_id = ?1
                ORDER BY sequence ASC
                "#,
            )
            .map_err(|error| error.to_string())?;
        let raw_transitions = stmt
            .query_map(params![&inbox], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, String>(6)?,
                ))
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        let transitions = raw_transitions
            .into_iter()
            .map(|(id, sequence, state, turn, incident, rebuilt, occurred)| {
                Ok(receipt::Transition {
                    id,
                    sequence,
                    state: receipt_state_from_db(&state)?,
                    turn,
                    incident,
                    rebuilt,
                    occurred,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let effects = Database::new(&conn).effects_for_receipt(&inbox)?;
        Ok(Some(receipt::Status {
            inbox,
            strand,
            state: receipt_state_from_db(&state)?,
            accepted,
            updated,
            transitions,
            effects,
        }))
    }
}
