use std::collections::BTreeSet;

use rusqlite::{Connection, OptionalExtension, params};

use super::super::SantiStore;
use crate::{ReceiptState, ReceiptStatus, ReceiptTransition, prefixed_id, timestamp_now};

pub(in crate::store) fn insert_accepted_in_conn(
    conn: &Connection,
    inbox_id: &str,
    strand_id: &str,
    accepted_at: &str,
) -> Result<(), String> {
    conn.execute(
        r#"
        INSERT INTO inbox_receipts (id, strand_id, state, accepted_at, updated_at)
        VALUES (?1, ?2, 'accepted', ?3, ?3)
        "#,
        params![inbox_id, strand_id, accepted_at],
    )
    .map_err(|error| error.to_string())?;
    append_transition(
        conn,
        inbox_id,
        ReceiptState::Accepted,
        None,
        None,
        accepted_at,
    )
}

pub(in crate::store) fn begin_turn_in_conn(
    conn: &Connection,
    strand_id: &str,
    turn_id: &str,
    drained_inbox_ids: &[String],
    recovered_incident_id: Option<&str>,
) -> Result<(), String> {
    let mut receipt_ids = drained_inbox_ids.iter().cloned().collect::<BTreeSet<_>>();
    let mut stmt = conn
        .prepare("SELECT id FROM inbox_receipts WHERE strand_id = ?1 AND state = 'turn_failed'")
        .map_err(|error| error.to_string())?;
    let failed = stmt
        .query_map(params![strand_id], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?;
    for receipt_id in failed {
        receipt_ids.insert(receipt_id.map_err(|error| error.to_string())?);
    }
    drop(stmt);

    let now = timestamp_now();
    if let Some(incident_id) = recovered_incident_id {
        for inbox_id in drained_inbox_ids {
            set_state(conn, inbox_id, ReceiptState::MechanicallyRecovered, &now)?;
            append_transition(
                conn,
                inbox_id,
                ReceiptState::MechanicallyRecovered,
                Some(turn_id),
                Some(incident_id),
                &now,
            )?;
        }
    }
    for inbox_id in receipt_ids {
        set_state(conn, &inbox_id, ReceiptState::Driving, &now)?;
        append_transition(
            conn,
            &inbox_id,
            ReceiptState::Driving,
            Some(turn_id),
            recovered_incident_id,
            &now,
        )?;
    }
    Ok(())
}

pub(in crate::store) fn fail_turn_in_conn(
    conn: &Connection,
    turn_id: &str,
    incident_id: Option<&str>,
    occurred_at: &str,
) -> Result<(), String> {
    transition_turn_receipts(
        conn,
        turn_id,
        ReceiptState::TurnFailed,
        incident_id,
        occurred_at,
    )
}

pub(in crate::store) fn complete_turn_in_conn(
    conn: &Connection,
    turn_id: &str,
    occurred_at: &str,
) -> Result<(), String> {
    transition_turn_receipts(conn, turn_id, ReceiptState::Completed, None, occurred_at)
}

fn transition_turn_receipts(
    conn: &Connection,
    turn_id: &str,
    state: ReceiptState,
    incident_id: Option<&str>,
    occurred_at: &str,
) -> Result<(), String> {
    let mut stmt = conn
        .prepare(
            r#"
            SELECT DISTINCT transition.inbox_id
            FROM receipt_transitions AS transition
            JOIN inbox_receipts AS receipt ON receipt.id = transition.inbox_id
            WHERE transition.turn_id = ?1
              AND transition.state = 'driving'
              AND receipt.state IN ('driving', 'mechanically_recovered')
            "#,
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(params![turn_id], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?;
    let mut receipt_ids = Vec::new();
    for row in rows {
        receipt_ids.push(row.map_err(|error| error.to_string())?);
    }
    drop(stmt);
    for inbox_id in receipt_ids {
        set_state(conn, &inbox_id, state.clone(), occurred_at)?;
        append_transition(
            conn,
            &inbox_id,
            state.clone(),
            Some(turn_id),
            incident_id,
            occurred_at,
        )?;
    }
    Ok(())
}

fn set_state(
    conn: &Connection,
    inbox_id: &str,
    state: ReceiptState,
    updated_at: &str,
) -> Result<(), String> {
    conn.execute(
        "UPDATE inbox_receipts SET state = ?2, updated_at = ?3 WHERE id = ?1",
        params![inbox_id, receipt_state_db(&state), updated_at],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn append_transition(
    conn: &Connection,
    inbox_id: &str,
    state: ReceiptState,
    turn_id: Option<&str>,
    incident_id: Option<&str>,
    occurred_at: &str,
) -> Result<(), String> {
    conn.execute(
        r#"
        INSERT INTO receipt_transitions (
          id, inbox_id, sequence, state, turn_id, incident_id, occurred_at
        )
        SELECT ?1, ?2, COALESCE(MAX(sequence), 0) + 1, ?3, ?4, ?5, ?6
        FROM receipt_transitions WHERE inbox_id = ?2
        "#,
        params![
            prefixed_id("rct"),
            inbox_id,
            receipt_state_db(&state),
            turn_id,
            incident_id,
            occurred_at,
        ],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

impl SantiStore {
    pub fn receipt_status(&self, inbox_id: &str) -> Result<Option<ReceiptStatus>, String> {
        let conn = self.conn.lock().unwrap();
        let receipt = conn
            .query_row(
                r#"
                SELECT id, strand_id, state, accepted_at, updated_at
                FROM inbox_receipts WHERE id = ?1
                "#,
                params![inbox_id],
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
        let Some((inbox_id, strand_id, state, accepted_at, updated_at)) = receipt else {
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
            .query_map(params![&inbox_id], |row| {
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
            .map(
                |(id, sequence, state, turn_id, incident_id, reconstructed_from, occurred_at)| {
                    Ok(ReceiptTransition {
                        id,
                        sequence,
                        state: receipt_state_from_db(&state)?,
                        turn_id,
                        incident_id,
                        reconstructed_from,
                        occurred_at,
                    })
                },
            )
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Some(ReceiptStatus {
            inbox_id,
            strand_id,
            state: receipt_state_from_db(&state)?,
            accepted_at,
            updated_at,
            transitions,
        }))
    }
}

fn receipt_state_db(state: &ReceiptState) -> &'static str {
    match state {
        ReceiptState::Accepted => "accepted",
        ReceiptState::MechanicallyRecovered => "mechanically_recovered",
        ReceiptState::Driving => "driving",
        ReceiptState::TurnFailed => "turn_failed",
        ReceiptState::Completed => "completed",
    }
}

fn receipt_state_from_db(state: &str) -> Result<ReceiptState, String> {
    match state {
        "accepted" => Ok(ReceiptState::Accepted),
        "mechanically_recovered" => Ok(ReceiptState::MechanicallyRecovered),
        "driving" => Ok(ReceiptState::Driving),
        "turn_failed" => Ok(ReceiptState::TurnFailed),
        "completed" => Ok(ReceiptState::Completed),
        _ => Err(format!("unknown receipt state: {state}")),
    }
}
