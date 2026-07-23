use std::collections::BTreeSet;

use rusqlite::params;

use super::Database;
use santi_model::{ReceiptState, now, tag};

struct Transition<'a> {
    state: ReceiptState,
    turn: Option<&'a str>,
    incident: Option<&'a str>,
    time: &'a str,
}

impl Database<'_> {
    pub fn insert_accepted(&self, inbox: &str, strand: &str, accepted: &str) -> Result<(), String> {
        self.conn
            .execute(
                r#"
        INSERT INTO inbox_receipts (id, strand_id, state, accepted_at, updated_at)
        VALUES (?1, ?2, 'accepted', ?3, ?3)
        "#,
                params![inbox, strand, accepted],
            )
            .map_err(|error| error.to_string())?;
        self.append_transition(
            inbox,
            Transition {
                state: ReceiptState::Accepted,
                turn: None,
                incident: None,
                time: accepted,
            },
        )
    }

    pub fn begin_turn(
        &self,
        strand: &str,
        turn: &str,
        drained_inbox_ids: &[String],
        recovered_incident_id: Option<&str>,
    ) -> Result<(), String> {
        let mut receipts = drained_inbox_ids.iter().cloned().collect::<BTreeSet<_>>();
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM inbox_receipts WHERE strand_id = ?1 AND state = 'turn_failed'")
            .map_err(|error| error.to_string())?;
        let failed = stmt
            .query_map(params![strand], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?;
        for receipt_id in failed {
            receipts.insert(receipt_id.map_err(|error| error.to_string())?);
        }
        drop(stmt);

        let now = now();
        if let Some(incident) = recovered_incident_id {
            for inbox in drained_inbox_ids {
                self.set_state(inbox, ReceiptState::MechanicallyRecovered, &now)?;
                self.append_transition(
                    inbox,
                    Transition {
                        state: ReceiptState::MechanicallyRecovered,
                        turn: Some(turn),
                        incident: Some(incident),
                        time: &now,
                    },
                )?;
            }
        }
        for inbox in receipts {
            self.set_state(&inbox, ReceiptState::Driving, &now)?;
            self.append_transition(
                &inbox,
                Transition {
                    state: ReceiptState::Driving,
                    turn: Some(turn),
                    incident: recovered_incident_id,
                    time: &now,
                },
            )?;
        }
        Ok(())
    }

    pub fn fail_turn(
        &self,
        turn: &str,
        incident: Option<&str>,
        occurred: &str,
    ) -> Result<(), String> {
        self.transition_turn_receipts(turn, ReceiptState::TurnFailed, incident, occurred)
    }

    pub fn complete_turn(&self, turn: &str, occurred_at: &str) -> Result<(), String> {
        self.transition_turn_receipts(turn, ReceiptState::Completed, None, occurred_at)
    }

    fn transition_turn_receipts(
        &self,
        turn: &str,
        state: ReceiptState,
        incident: Option<&str>,
        occurred: &str,
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
            .query_map(params![turn], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?;
        let mut receipts = Vec::new();
        for row in rows {
            receipts.push(row.map_err(|error| error.to_string())?);
        }
        drop(stmt);
        for inbox in receipts {
            self.set_state(&inbox, state.clone(), occurred)?;
            self.append_transition(
                &inbox,
                Transition {
                    state: state.clone(),
                    turn: Some(turn),
                    incident,
                    time: occurred,
                },
            )?;
        }
        Ok(())
    }

    fn set_state(&self, inbox: &str, state: ReceiptState, updated: &str) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE inbox_receipts SET state = ?2, updated_at = ?3 WHERE id = ?1",
                params![inbox, receipt_state_db(&state), updated],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn append_transition(&self, inbox: &str, transition: Transition<'_>) -> Result<(), String> {
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
                    tag("rct"),
                    inbox,
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

pub fn receipt_state_db(state: &ReceiptState) -> &'static str {
    match state {
        ReceiptState::Accepted => "accepted",
        ReceiptState::MechanicallyRecovered => "mechanically_recovered",
        ReceiptState::Driving => "driving",
        ReceiptState::TurnFailed => "turn_failed",
        ReceiptState::Completed => "completed",
    }
}

pub fn receipt_state_from_db(state: &str) -> Result<ReceiptState, String> {
    match state {
        "accepted" => Ok(ReceiptState::Accepted),
        "mechanically_recovered" => Ok(ReceiptState::MechanicallyRecovered),
        "driving" => Ok(ReceiptState::Driving),
        "turn_failed" => Ok(ReceiptState::TurnFailed),
        "completed" => Ok(ReceiptState::Completed),
        _ => Err(format!("unknown receipt state: {state}")),
    }
}
