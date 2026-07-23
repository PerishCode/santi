use std::collections::BTreeSet;

use rusqlite::params;

use super::Database;
use santi_model::receipt;
use santi_model::{now, tag};

struct Transition<'a> {
    state: receipt::State,
    turn: Option<&'a str>,
    incident: Option<&'a str>,
    time: &'a str,
}

impl Database<'_> {
    pub fn accept(&self, inbox: &str, strand: &str, accepted: &str) -> Result<(), String> {
        self.conn
            .execute(
                r#"
        INSERT INTO inbox_receipts (id, strand_id, state, accepted_at, updated_at)
        VALUES (?1, ?2, 'accepted', ?3, ?3)
        "#,
                params![inbox, strand, accepted],
            )
            .map_err(|error| error.to_string())?;
        self.noted(
            inbox,
            Transition {
                state: receipt::State::Accepted,
                turn: None,
                incident: None,
                time: accepted,
            },
        )
    }

    pub fn begin(
        &self,
        strand: &str,
        turn: &str,
        drained_inbox_ids: &[String],
        recovered: Option<&str>,
    ) -> Result<(), String> {
        let mut receipts = drained_inbox_ids.iter().cloned().collect::<BTreeSet<_>>();
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM inbox_receipts WHERE strand_id = ?1 AND state = 'failed'")
            .map_err(|error| error.to_string())?;
        let failed = stmt
            .query_map(params![strand], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?;
        for receipt_id in failed {
            receipts.insert(receipt_id.map_err(|error| error.to_string())?);
        }
        drop(stmt);

        let now = now();
        if let Some(incident) = recovered {
            for inbox in drained_inbox_ids {
                self.shift(inbox, receipt::State::Recovered, &now)?;
                self.noted(
                    inbox,
                    Transition {
                        state: receipt::State::Recovered,
                        turn: Some(turn),
                        incident: Some(incident),
                        time: &now,
                    },
                )?;
            }
        }
        for inbox in receipts {
            self.shift(&inbox, receipt::State::Driving, &now)?;
            self.noted(
                &inbox,
                Transition {
                    state: receipt::State::Driving,
                    turn: Some(turn),
                    incident: recovered,
                    time: &now,
                },
            )?;
        }
        Ok(())
    }

    pub fn fail(&self, turn: &str, incident: Option<&str>, occurred: &str) -> Result<(), String> {
        self.close(turn, receipt::State::Failed, incident, occurred)
    }

    pub fn complete(&self, turn: &str, occurred_at: &str) -> Result<(), String> {
        self.close(turn, receipt::State::Completed, None, occurred_at)
    }

    fn close(
        &self,
        turn: &str,
        state: receipt::State,
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
              AND receipt.state IN ('driving', 'recovered')
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
            self.shift(&inbox, state.clone(), occurred)?;
            self.noted(
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

    fn shift(&self, inbox: &str, state: receipt::State, updated: &str) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE inbox_receipts SET state = ?2, updated_at = ?3 WHERE id = ?1",
                params![inbox, state.encode(), updated],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn noted(&self, inbox: &str, transition: Transition<'_>) -> Result<(), String> {
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
                    transition.state.encode(),
                    transition.turn,
                    transition.incident,
                    transition.time,
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }
}
