use rusqlite::{OptionalExtension, params};

use super::Database;
use crate::store::rows::{Decode, collect_rows};
use crate::{EffectState, EffectTransition, EffectTransitionReason, StrandEffect, prefixed_id};

const EFFECT_COLUMNS: &str = r#"
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
    pub state: EffectState,
    pub reason: EffectTransitionReason,
    pub evidence: Option<&'a str>,
    pub time: &'a str,
}

impl Database<'_> {
    pub fn insert_prepared(&self, prepared: Prepared<'_>) -> Result<String, String> {
        let effect_id = prefixed_id("effect");
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
                    effect_id,
                    prepared.strand,
                    prepared.turn,
                    prepared.call,
                    prepared.kind,
                    prepared.time,
                ],
            )
            .map_err(|error| error.to_string())?;
        self.append_effect_transition(
            &effect_id,
            Transition {
                state: EffectState::Prepared,
                reason: EffectTransitionReason::IntentPersisted,
                evidence: None,
                time: prepared.time,
            },
        )?;
        Ok(effect_id)
    }

    pub fn append_effect_transition(
        &self,
        effect_id: &str,
        transition: Transition<'_>,
    ) -> Result<(), String> {
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
                    prefixed_id("efx"),
                    effect_id,
                    transition.state.encode(),
                    transition.reason.encode(),
                    transition.evidence,
                    transition.time,
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn find_effect(&self, effect_id: &str) -> Result<Option<StrandEffect>, String> {
        self.conn
            .query_row(
                &format!("SELECT {EFFECT_COLUMNS} FROM strand_effects WHERE id = ?1 LIMIT 1"),
                params![effect_id],
                StrandEffect::decode,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub fn effects_for_receipt(&self, inbox_id: &str) -> Result<Vec<StrandEffect>, String> {
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
            .query_map(params![inbox_id], StrandEffect::decode)
            .map_err(|error| error.to_string())?;
        collect_rows(rows)
    }

    pub fn effect_transitions(&self, effect_id: &str) -> Result<Vec<EffectTransition>, String> {
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
            .query_map(params![effect_id], |row| {
                Ok(EffectTransition {
                    id: row.get(0)?,
                    sequence: row.get(1)?,
                    state: EffectState::decode(&row.get::<_, String>(2)?),
                    reason: EffectTransitionReason::decode(&row.get::<_, String>(3)?),
                    evidence: row.get(4)?,
                    occurred_at: row.get(5)?,
                })
            })
            .map_err(|error| error.to_string())?;
        collect_rows(rows)
    }

    pub fn receipt_ids(&self, effect_id: &str) -> Result<Vec<String>, String> {
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
            .query_map(params![effect_id], |row| row.get(0))
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    pub fn reconcile_effects(
        &self,
        turn_id: &str,
        prepared_reason: EffectTransitionReason,
        dispatching_reason: EffectTransitionReason,
        occurred_at: &str,
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
            .query_map(params![turn_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        drop(stmt);

        for (effect_id, state) in rows {
            let (next, reason, settled) = if state == "prepared" {
                (EffectState::NotDispatched, prepared_reason.clone(), true)
            } else {
                (EffectState::Unknown, dispatching_reason.clone(), false)
            };
            self.conn
                .execute(
                    r#"
            UPDATE strand_effects
            SET state = ?2, updated_at = ?3,
                settled_at = CASE WHEN ?4 THEN ?3 ELSE settled_at END
            WHERE id = ?1
            "#,
                    params![effect_id, next.encode(), occurred_at, settled],
                )
                .map_err(|error| error.to_string())?;
            self.append_effect_transition(
                &effect_id,
                Transition {
                    state: next,
                    reason,
                    evidence: None,
                    time: occurred_at,
                },
            )?;
        }
        Ok(())
    }
}
