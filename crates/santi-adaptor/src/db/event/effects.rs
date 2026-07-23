use rusqlite::{OptionalExtension, params};

use super::Database;
use crate::rows::{Decode, collected};
use santi_model::effect;
use santi_model::tag;

const COLUMNS: &str = r#"
    id, strand_id, turn_id, tool_call_id, effect_type, state,
    result_ref, error_text, created_at, updated_at, dispatched_at, settled_at
"#;

pub struct Prepared<'a> {
    pub strand: &'a str,
    pub turn: &'a str,
    pub call: &'a str,
    pub kind: &'a str,
    pub time: &'a str,
}

pub struct Transition<'a> {
    pub state: effect::State,
    pub reason: effect::Reason,
    pub evidence: Option<&'a str>,
    pub time: &'a str,
}

impl Database<'_> {
    pub fn prepare(&self, prepared: Prepared<'_>) -> Result<String, String> {
        let effect = tag("effect");
        self.conn
            .execute(
                r#"
        INSERT INTO strand_effects (
          id, strand_id, turn_id, tool_call_id, effect_type, state,
          result_ref, error_text, metadata, created_at, updated_at,
          dispatched_at, settled_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, 'prepared', NULL, NULL, NULL, ?6, ?6, NULL, NULL)
        "#,
                params![
                    effect,
                    prepared.strand,
                    prepared.turn,
                    prepared.call,
                    prepared.kind,
                    prepared.time,
                ],
            )
            .map_err(|error| error.to_string())?;
        self.shifted(
            &effect,
            Transition {
                state: effect::State::Prepared,
                reason: effect::Reason::IntentPersisted,
                evidence: None,
                time: prepared.time,
            },
        )?;
        Ok(effect)
    }

    pub fn shifted(&self, effect: &str, transition: Transition<'_>) -> Result<(), String> {
        self.conn
            .execute(
                r#"
        INSERT INTO effect_transitions (
          id, effect_id, sequence, state, reason, evidence, occurred_at
        )
        SELECT ?1, ?2, COALESCE(MAX(sequence), 0) + 1, ?3, ?4, ?5, ?6
        FROM effect_transitions WHERE effect_id = ?2
        "#,
                params![
                    tag("efx"),
                    effect,
                    transition.state.encode(),
                    transition.reason.encode(),
                    transition.evidence,
                    transition.time,
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn effect(&self, effect: &str) -> Result<Option<effect::Effect>, String> {
        self.conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM strand_effects WHERE id = ?1 LIMIT 1"),
                params![effect],
                effect::Effect::decode,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub fn effected(&self, inbox: &str) -> Result<Vec<effect::Effect>, String> {
        let mut stmt = self
            .conn
            .prepare(
                r#"
            SELECT DISTINCT
              effect.id, effect.strand_id, effect.turn_id, effect.tool_call_id,
              effect.effect_type, effect.state, effect.result_ref, effect.error_text,
              effect.created_at, effect.updated_at, effect.dispatched_at, effect.settled_at
            FROM strand_effects AS effect
            JOIN receipt_transitions AS receipt ON receipt.turn_id = effect.turn_id
            WHERE receipt.inbox_id = ?1
            ORDER BY effect.created_at, effect.id
            "#,
            )
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map(params![inbox], effect::Effect::decode)
            .map_err(|error| error.to_string())?;
        collected(rows)
    }

    pub fn shifts(&self, effect: &str) -> Result<Vec<effect::Transition>, String> {
        let mut stmt = self
            .conn
            .prepare(
                r#"
            SELECT id, sequence, state, reason, evidence, occurred_at
            FROM effect_transitions
            WHERE effect_id = ?1
            ORDER BY sequence
            "#,
            )
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map(params![effect], |row| {
                Ok(effect::Transition {
                    id: row.get(0)?,
                    sequence: row.get(1)?,
                    state: effect::State::decode(&row.get::<_, String>(2)?),
                    reason: effect::Reason::decode(&row.get::<_, String>(3)?),
                    evidence: row.get(4)?,
                    occurred: row.get(5)?,
                })
            })
            .map_err(|error| error.to_string())?;
        collected(rows)
    }

    pub fn receipts(&self, effect: &str) -> Result<Vec<String>, String> {
        let mut stmt = self
            .conn
            .prepare(
                r#"
            SELECT DISTINCT receipt.inbox_id
            FROM receipt_transitions AS receipt
            JOIN strand_effects AS effect ON effect.turn_id = receipt.turn_id
            WHERE effect.id = ?1
            ORDER BY receipt.inbox_id
            "#,
            )
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map(params![effect], |row| row.get(0))
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    pub fn reconcile(
        &self,
        turn: &str,
        prepared_reason: effect::Reason,
        dispatching_reason: effect::Reason,
        occurred: &str,
    ) -> Result<(), String> {
        let mut stmt = self
            .conn
            .prepare(
                r#"
            SELECT id, state FROM strand_effects
            WHERE turn_id = ?1 AND state IN ('prepared', 'dispatching')
            ORDER BY created_at, id
            "#,
            )
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map(params![turn], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        drop(stmt);

        for (effect, state) in rows {
            let (next, reason, settled) = if state == "prepared" {
                (effect::State::NotDispatched, prepared_reason.clone(), true)
            } else {
                (effect::State::Unknown, dispatching_reason.clone(), false)
            };
            self.conn
                .execute(
                    r#"
            UPDATE strand_effects
            SET state = ?2, updated_at = ?3,
                settled_at = CASE WHEN ?4 THEN ?3 ELSE settled_at END
            WHERE id = ?1
            "#,
                    params![effect, next.encode(), occurred, settled],
                )
                .map_err(|error| error.to_string())?;
            self.shifted(
                &effect,
                Transition {
                    state: next,
                    reason,
                    evidence: None,
                    time: occurred,
                },
            )?;
        }
        Ok(())
    }
}
