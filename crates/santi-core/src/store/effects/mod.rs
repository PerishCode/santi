use rusqlite::{OptionalExtension, params};
use serde_json::Value;

use super::{SantiStore, db::Database, rows::Decode};
use crate::{
    EffectResolutionOutcome, EffectState, EffectStatus, EffectTransition, EffectTransitionReason,
    StrandEffect, StrandTargetType, ToolResult, prefixed_id, timestamp_now,
};

const EFFECT_COLUMNS: &str = r#"
    id, strand_id, turn_id, tool_call_id, effect_type, state,
    result_ref, error_text, created_at, updated_at, dispatched_at, settled_at
"#;

pub(super) struct Prepared<'a> {
    pub(super) strand: &'a str,
    pub(super) turn: &'a str,
    pub(super) call: &'a str,
    pub(super) kind: &'a str,
    pub(super) time: &'a str,
}

struct Transition<'a> {
    state: EffectState,
    reason: EffectTransitionReason,
    evidence: Option<&'a str>,
    time: &'a str,
}

pub(crate) struct Settlement<'a> {
    pub(crate) call: &'a str,
    pub(crate) output: Option<Value>,
    pub(crate) error: Option<String>,
    pub(crate) state: EffectState,
}

impl Database<'_> {
    pub(in crate::store) fn insert_prepared(
        &self,
        prepared: Prepared<'_>,
    ) -> Result<String, String> {
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

    fn append_effect_transition(
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

    pub(in crate::store) fn find_effect(
        &self,
        effect_id: &str,
    ) -> Result<Option<StrandEffect>, String> {
        self.conn
            .query_row(
                &format!("SELECT {EFFECT_COLUMNS} FROM strand_effects WHERE id = ?1 LIMIT 1"),
                params![effect_id],
                StrandEffect::decode,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub(in crate::store) fn effects_for_receipt(
        &self,
        inbox_id: &str,
    ) -> Result<Vec<StrandEffect>, String> {
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
        super::rows::collect_rows(rows)
    }

    fn effect_transitions(&self, effect_id: &str) -> Result<Vec<EffectTransition>, String> {
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
        super::rows::collect_rows(rows)
    }

    fn receipt_ids(&self, effect_id: &str) -> Result<Vec<String>, String> {
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

    pub(in crate::store) fn reconcile_effects(
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

impl SantiStore {
    pub fn effect_status(&self, effect_id: &str) -> Result<Option<EffectStatus>, String> {
        let conn = self.conn.lock().unwrap();
        let database = Database::new(&conn);
        let Some(effect) = database.find_effect(effect_id)? else {
            return Ok(None);
        };
        Ok(Some(EffectStatus {
            transitions: database.effect_transitions(effect_id)?,
            receipt_ids: database.receipt_ids(effect_id)?,
            effect,
        }))
    }

    pub fn begin_effect_dispatch(&self, effect_id: &str) -> Result<StrandEffect, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let now = timestamp_now();
        let changed = tx
            .execute(
                r#"
                UPDATE strand_effects
                SET state = 'dispatching', updated_at = ?2, dispatched_at = ?2
                WHERE id = ?1 AND state = 'prepared'
                "#,
                params![effect_id, now],
            )
            .map_err(|error| error.to_string())?;
        if changed != 1 {
            return Err("effect is not prepared for dispatch".to_string());
        }
        Database::new(&tx).append_effect_transition(
            effect_id,
            Transition {
                state: EffectState::Dispatching,
                reason: EffectTransitionReason::DispatchWindowOpened,
                evidence: None,
                time: &now,
            },
        )?;
        tx.commit().map_err(|error| error.to_string())?;
        Database::new(&conn)
            .find_effect(effect_id)?
            .ok_or_else(|| "dispatching effect missing".to_string())
    }

    pub(crate) fn append_effect_tool_result(
        &self,
        effect_id: &str,
        settlement: Settlement<'_>,
    ) -> Result<ToolResult, String> {
        let Settlement {
            call: tool_call_id,
            output,
            error: error_text,
            state,
        } = settlement;
        let (allowed_source, reason) = match state {
            EffectState::Confirmed => ("dispatching", EffectTransitionReason::ResultPersisted),
            EffectState::NotDispatched => (
                "prepared_or_dispatching",
                EffectTransitionReason::DispatchRejected,
            ),
            _ => return Err("invalid terminal effect state for a tool result".to_string()),
        };
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let current: (String, Option<String>) = tx
            .query_row(
                "SELECT state, tool_call_id FROM strand_effects WHERE id = ?1",
                params![effect_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|error| error.to_string())?;
        let state_allowed = current.0 == allowed_source
            || (allowed_source == "prepared_or_dispatching"
                && matches!(current.0.as_str(), "prepared" | "dispatching"));
        if !state_allowed || current.1.as_deref() != Some(tool_call_id) {
            return Err("effect/tool-call state mismatch".to_string());
        }

        let tool_result_id = prefixed_id("tool_result");
        let now = timestamp_now();
        let database = Database::new(&tx);
        let strand_id = database.call_soul_id(tool_call_id)?;
        let output_text = output
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| error.to_string())?;
        tx.execute(
            r#"
            INSERT INTO tool_results (id, tool_call_id, output, error_text, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
            params![tool_result_id, tool_call_id, output_text, error_text, now],
        )
        .map_err(|error| error.to_string())?;
        database.append_entry_in_tx(&strand_id, StrandTargetType::ToolResult, &tool_result_id)?;
        tx.execute(
            r#"
            UPDATE strand_effects
            SET state = ?2, result_ref = ?3, error_text = ?4,
                updated_at = ?5, settled_at = ?5
            WHERE id = ?1
            "#,
            params![effect_id, state.encode(), tool_result_id, error_text, now,],
        )
        .map_err(|error| error.to_string())?;
        database.append_effect_transition(
            effect_id,
            Transition {
                state,
                reason,
                evidence: Some(&format!("tool_result:{tool_result_id}")),
                time: &now,
            },
        )?;
        tx.commit().map_err(|error| error.to_string())?;
        Database::new(&conn)
            .tool_result_by_id(&tool_result_id)?
            .ok_or_else(|| "created effect tool_result missing".to_string())
    }

    pub fn mark_effect_unknown(
        &self,
        effect_id: &str,
        reason: EffectTransitionReason,
        evidence: &str,
    ) -> Result<StrandEffect, String> {
        if !matches!(reason, EffectTransitionReason::ResultCaptureFailed) {
            return Err("invalid live unknown-effect reason".to_string());
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let now = timestamp_now();
        let changed = tx
            .execute(
                r#"
                UPDATE strand_effects
                SET state = 'unknown', error_text = ?2, updated_at = ?3
                WHERE id = ?1 AND state = 'dispatching'
                "#,
                params![effect_id, evidence, now],
            )
            .map_err(|error| error.to_string())?;
        if changed != 1 {
            return Err("effect is not dispatching".to_string());
        }
        Database::new(&tx).append_effect_transition(
            effect_id,
            Transition {
                state: EffectState::Unknown,
                reason,
                evidence: Some(evidence),
                time: &now,
            },
        )?;
        tx.commit().map_err(|error| error.to_string())?;
        Database::new(&conn)
            .find_effect(effect_id)?
            .ok_or_else(|| "unknown effect missing".to_string())
    }

    pub fn resolve_effect(
        &self,
        effect_id: &str,
        outcome: EffectResolutionOutcome,
        evidence: &str,
    ) -> Result<Option<EffectStatus>, String> {
        let evidence = evidence.trim();
        if evidence.is_empty() {
            return Err("effect resolution evidence must not be empty".to_string());
        }
        let (state, reason) = match outcome {
            EffectResolutionOutcome::Applied => (
                EffectState::ResolvedApplied,
                EffectTransitionReason::OperatorResolvedApplied,
            ),
            EffectResolutionOutcome::NotApplied => (
                EffectState::ResolvedNotApplied,
                EffectTransitionReason::OperatorResolvedNotApplied,
            ),
        };
        let mut conn = self.conn.lock().unwrap();
        if Database::new(&conn).find_effect(effect_id)?.is_none() {
            return Ok(None);
        }
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let now = timestamp_now();
        let changed = tx
            .execute(
                r#"
                UPDATE strand_effects
                SET state = ?2, updated_at = ?3, settled_at = ?3
                WHERE id = ?1 AND state = 'unknown'
                "#,
                params![effect_id, state.encode(), now],
            )
            .map_err(|error| error.to_string())?;
        if changed != 1 {
            return Err("only an unknown effect can be resolved".to_string());
        }
        Database::new(&tx).append_effect_transition(
            effect_id,
            Transition {
                state,
                reason,
                evidence: Some(evidence),
                time: &now,
            },
        )?;
        tx.commit().map_err(|error| error.to_string())?;
        drop(conn);
        self.effect_status(effect_id)
    }
}
