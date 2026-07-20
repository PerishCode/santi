use std::collections::BTreeSet;

use rusqlite::params;

use super::Database;
use crate::{ReceiptState, prefixed_id, timestamp_now};

struct Transition<'a> {
    state: ReceiptState,
    turn: Option<&'a str>,
    incident: Option<&'a str>,
    time: &'a str,
}

impl Database<'_> {
    pub(in crate::store) fn insert_accepted(
        &self,
        inbox_id: &str,
        strand_id: &str,
        accepted_at: &str,
    ) -> Result<(), String> {
        self.conn
            .execute(
                r#"
        INSERT INTO inbox_receipts (id, strand_id, state, accepted_at, updated_at)
        VALUES (?1, ?2, 'accepted', ?3, ?3)
        "#,
                params![inbox_id, strand_id, accepted_at],
            )
            .map_err(|error| error.to_string())?;
        self.append_transition(
            inbox_id,
            Transition {
                state: ReceiptState::Accepted,
                turn: None,
                incident: None,
                time: accepted_at,
            },
        )
    }

    pub(in crate::store) fn begin_turn(
        &self,
        strand_id: &str,
        turn_id: &str,
        drained_inbox_ids: &[String],
        recovered_incident_id: Option<&str>,
    ) -> Result<(), String> {
        let mut receipt_ids = drained_inbox_ids.iter().cloned().collect::<BTreeSet<_>>();
        let mut stmt = self
            .conn
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
                self.set_state(inbox_id, ReceiptState::MechanicallyRecovered, &now)?;
                self.append_transition(
                    inbox_id,
                    Transition {
                        state: ReceiptState::MechanicallyRecovered,
                        turn: Some(turn_id),
                        incident: Some(incident_id),
                        time: &now,
                    },
                )?;
            }
        }
        for inbox_id in receipt_ids {
            self.set_state(&inbox_id, ReceiptState::Driving, &now)?;
            self.append_transition(
                &inbox_id,
                Transition {
                    state: ReceiptState::Driving,
                    turn: Some(turn_id),
                    incident: recovered_incident_id,
                    time: &now,
                },
            )?;
        }
        Ok(())
    }

    pub(in crate::store) fn fail_turn(
        &self,
        turn_id: &str,
        incident_id: Option<&str>,
        occurred_at: &str,
    ) -> Result<(), String> {
        self.transition_turn_receipts(turn_id, ReceiptState::TurnFailed, incident_id, occurred_at)
    }

    pub(in crate::store) fn complete_turn(
        &self,
        turn_id: &str,
        occurred_at: &str,
    ) -> Result<(), String> {
        self.transition_turn_receipts(turn_id, ReceiptState::Completed, None, occurred_at)
    }

    fn transition_turn_receipts(
        &self,
        turn_id: &str,
        state: ReceiptState,
        incident_id: Option<&str>,
        occurred_at: &str,
    ) -> Result<(), String> {
        let mut stmt = self
            .conn
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
            self.set_state(&inbox_id, state.clone(), occurred_at)?;
            self.append_transition(
                &inbox_id,
                Transition {
                    state: state.clone(),
                    turn: Some(turn_id),
                    incident: incident_id,
                    time: occurred_at,
                },
            )?;
        }
        Ok(())
    }

    fn set_state(
        &self,
        inbox_id: &str,
        state: ReceiptState,
        updated_at: &str,
    ) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE inbox_receipts SET state = ?2, updated_at = ?3 WHERE id = ?1",
                params![inbox_id, receipt_state_db(&state), updated_at],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn append_transition(&self, inbox_id: &str, transition: Transition<'_>) -> Result<(), String> {
        self.conn
            .execute(
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
                    receipt_state_db(&transition.state),
                    transition.turn,
                    transition.incident,
                    transition.time,
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }
}

pub(crate) fn receipt_state_db(state: &ReceiptState) -> &'static str {
    match state {
        ReceiptState::Accepted => "accepted",
        ReceiptState::MechanicallyRecovered => "mechanically_recovered",
        ReceiptState::Driving => "driving",
        ReceiptState::TurnFailed => "turn_failed",
        ReceiptState::Completed => "completed",
    }
}

pub(crate) fn receipt_state_from_db(state: &str) -> Result<ReceiptState, String> {
    match state {
        "accepted" => Ok(ReceiptState::Accepted),
        "mechanically_recovered" => Ok(ReceiptState::MechanicallyRecovered),
        "driving" => Ok(ReceiptState::Driving),
        "turn_failed" => Ok(ReceiptState::TurnFailed),
        "completed" => Ok(ReceiptState::Completed),
        _ => Err(format!("unknown receipt state: {state}")),
    }
}
