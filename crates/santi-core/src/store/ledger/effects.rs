use serde_json::Value;
pub(crate) struct Settlement<'a> {
    pub(crate) call: &'a str,
    pub(crate) output: Option<Value>,
    pub(crate) error: Option<String>,
    pub(crate) state: effect::State,
}

use crate::store::Store;
use crate::store::db::{Database, Transition};
use crate::{effect, strand, tool};
use crate::{now, tag};
use rusqlite::params;

impl Store {
    pub fn effect(&self, effect: &str) -> Result<Option<effect::Status>, String> {
        let conn = self.conn.lock().unwrap();
        let database = Database::new(&conn);
        let Some(held) = database.effect(effect)? else {
            return Ok(None);
        };
        Ok(Some(effect::Status {
            transitions: database.shifts(effect)?,
            receipts: database.receipts(effect)?,
            effect: held,
        }))
    }

    pub fn dispatch(&self, effect: &str) -> Result<effect::Effect, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let now = now();
        let changed = tx
            .execute(
                r#"
                UPDATE strand_effects
                SET state = 'dispatching', updated_at = ?2, dispatched_at = ?2
                WHERE id = ?1 AND state = 'prepared'
                "#,
                params![effect, now],
            )
            .map_err(|error| error.to_string())?;
        if changed != 1 {
            return Err("effect is not prepared for dispatch".to_string());
        }
        Database::new(&tx).shifted(
            effect,
            Transition {
                state: effect::State::Dispatching,
                reason: effect::Reason::DispatchWindowOpened,
                evidence: None,
                time: &now,
            },
        )?;
        tx.commit().map_err(|error| error.to_string())?;
        Database::new(&conn)
            .effect(effect)?
            .ok_or_else(|| "dispatching effect missing".to_string())
    }

    pub(crate) fn append_effect_tool_result(
        &self,
        effect: &str,
        settlement: Settlement<'_>,
    ) -> Result<tool::Reply, String> {
        let Settlement {
            call,
            output,
            error,
            state,
        } = settlement;
        let (allowed_source, reason) = match state {
            effect::State::Confirmed => ("dispatching", effect::Reason::ResultPersisted),
            effect::State::NotDispatched => {
                ("prepared_or_dispatching", effect::Reason::DispatchRejected)
            }
            _ => return Err("invalid terminal effect state for a tool result".to_string()),
        };
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let current: (String, Option<String>) = tx
            .query_row(
                "SELECT state, tool_call_id FROM strand_effects WHERE id = ?1",
                params![effect],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|error| error.to_string())?;
        let allowed = current.0 == allowed_source
            || (allowed_source == "prepared_or_dispatching"
                && matches!(current.0.as_str(), "prepared" | "dispatching"));
        if !allowed || current.1.as_deref() != Some(call) {
            return Err("effect/tool-call state mismatch".to_string());
        }

        let reply = tag("tool_result");
        let now = now();
        let database = Database::new(&tx);
        let strand = database.caller(call)?;
        let output = output
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| error.to_string())?;
        tx.execute(
            r#"
            INSERT INTO tool_results (id, tool_call_id, output, error_text, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
            params![reply, call, output, error, now],
        )
        .map_err(|error| error.to_string())?;
        database.entered(&strand, strand::Target::ToolResult, &reply)?;
        tx.execute(
            r#"
            UPDATE strand_effects
            SET state = ?2, result_ref = ?3, error_text = ?4,
                updated_at = ?5, settled_at = ?5
            WHERE id = ?1
            "#,
            params![effect, state.encode(), reply, error, now,],
        )
        .map_err(|error| error.to_string())?;
        database.shifted(
            effect,
            Transition {
                state,
                reason,
                evidence: Some(&format!("tool_result:{reply}")),
                time: &now,
            },
        )?;
        tx.commit().map_err(|error| error.to_string())?;
        Database::new(&conn)
            .reply(&reply)?
            .ok_or_else(|| "created effect tool_result missing".to_string())
    }

    pub fn unmark(
        &self,
        effect: &str,
        reason: effect::Reason,
        evidence: &str,
    ) -> Result<effect::Effect, String> {
        if !matches!(reason, effect::Reason::ResultCaptureFailed) {
            return Err("invalid live unknown-effect reason".to_string());
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let now = now();
        let changed = tx
            .execute(
                r#"
                UPDATE strand_effects
                SET state = 'unknown', error_text = ?2, updated_at = ?3
                WHERE id = ?1 AND state = 'dispatching'
                "#,
                params![effect, evidence, now],
            )
            .map_err(|error| error.to_string())?;
        if changed != 1 {
            return Err("effect is not dispatching".to_string());
        }
        Database::new(&tx).shifted(
            effect,
            Transition {
                state: effect::State::Unknown,
                reason,
                evidence: Some(evidence),
                time: &now,
            },
        )?;
        tx.commit().map_err(|error| error.to_string())?;
        Database::new(&conn)
            .effect(effect)?
            .ok_or_else(|| "unknown effect missing".to_string())
    }

    pub fn settle(
        &self,
        effect: &str,
        outcome: effect::Outcome,
        evidence: &str,
    ) -> Result<Option<effect::Status>, String> {
        let evidence = evidence.trim();
        if evidence.is_empty() {
            return Err("effect resolution evidence must not be empty".to_string());
        }
        let (state, reason) = match outcome {
            effect::Outcome::Applied => (
                effect::State::ResolvedApplied,
                effect::Reason::OperatorResolvedApplied,
            ),
            effect::Outcome::NotApplied => (
                effect::State::ResolvedNotApplied,
                effect::Reason::OperatorResolvedNotApplied,
            ),
        };
        let mut conn = self.conn.lock().unwrap();
        if Database::new(&conn).effect(effect)?.is_none() {
            return Ok(None);
        }
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let now = now();
        let changed = tx
            .execute(
                r#"
                UPDATE strand_effects
                SET state = ?2, updated_at = ?3, settled_at = ?3
                WHERE id = ?1 AND state = 'unknown'
                "#,
                params![effect, state.encode(), now],
            )
            .map_err(|error| error.to_string())?;
        if changed != 1 {
            return Err("only an unknown effect can be resolved".to_string());
        }
        Database::new(&tx).shifted(
            effect,
            Transition {
                state,
                reason,
                evidence: Some(evidence),
                time: &now,
            },
        )?;
        tx.commit().map_err(|error| error.to_string())?;
        drop(conn);
        self.effect(effect)
    }
}
