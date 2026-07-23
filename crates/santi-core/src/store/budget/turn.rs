use super::*;
use crate::budget;

impl Store {
    pub(crate) fn budgeted(&self, launch: Launch<'_>) -> Result<Opened, String> {
        let Launch {
            strand,
            trigger,
            reference,
            admission,
            recover,
        } = launch;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let running: Option<String> = tx
            .query_row(
                "SELECT id FROM turns WHERE strand_id = ?1 AND status = 'running' LIMIT 1",
                params![strand],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        if let Some(turn) = running {
            let turn = Database::new(&tx)
                .turn(&turn)?
                .ok_or_else(|| "running turn missing".to_string())?;
            return Ok(Opened::Running(turn));
        }

        let database = Database::new(&tx);
        let pending = super::state::queued(&database, strand)?;
        let scarred = recover
            && tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM inbox_receipts WHERE strand_id = ?1 AND state = 'failed')",
                    params![strand],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|error| error.to_string())?
                != 0;
        if pending.is_empty() && !scarred {
            return Ok(Opened::Idle);
        }

        if database
            .incident(&catalog::CONTEXT_BUDGET_EXCEEDED.key("strand", strand))?
            .is_some()
        {
            let error = super::state::repress(&database, strand, "pending_active_guard")?;
            tx.commit().map_err(|error| error.to_string())?;
            return Ok(Opened::Held(error));
        }

        if let Some(admission) = admission {
            let mut input = assembled(&tx, strand)?;
            input.extend(pending);
            let estimate = crate::context::budget::estimated(
                &input,
                admission.instructions.as_deref(),
                Some(admission.tools.as_slice()),
            );
            if estimate.total > admission.bytes {
                let reason = reason(estimate.total, admission.bytes);
                let observed = database.cursor(strand)?;
                let error = super::state::press(
                    &database,
                    strand,
                    Pressure {
                        code: PENDING,
                        text: &reason,
                        operation: "pending_drain_admission",
                        provider: Some(&admission.provider),
                        model: Some(&admission.model),
                        source: Some(&admission.source),
                        bytes: Some(admission.bytes),
                        estimate: &estimate,
                        observed: None,
                        at: observed,
                        metadata: Some(json!({"estimator": estimate.estimator})),
                    },
                )?;
                tx.commit().map_err(|error| error.to_string())?;
                return Ok(Opened::Held(error));
            }
        }

        let turn = tag("turn");
        let drained = drain(&tx, strand, &turn)?;
        if drained.messages.is_empty() && !scarred {
            return Ok(Opened::Idle);
        }
        let now = now();
        tx.execute(
            r#"
            INSERT INTO turns (
              id, strand_id, trigger_type, trigger_ref,
              base_strand_seq, end_strand_seq, status, error_text,
              created_at, updated_at, finished_at
            )
            SELECT ?1, id, ?3, ?4, next_seq - 1, NULL, 'running', NULL, ?5, ?5, NULL
            FROM strands WHERE id = ?2
            "#,
            params![turn, strand, trigger, reference, now],
        )
        .map_err(|error| error.to_string())?;
        let recovered =
            crate::store::errors::drive::revive(&tx, strand, &turn, drained.messages.len())?;
        Database::new(&tx).begin(strand, &turn, &drained.inboxes, recovered.as_deref())?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(Opened::Started(Begun {
            turn: Database::new(&conn)
                .turn(&turn)?
                .ok_or_else(|| "created turn missing".to_string())?,
            drained: drained.messages,
        }))
    }
}

impl Store {
    pub(crate) fn spent(&self, strand: &str) -> Result<budget::Usage, String> {
        let conn = self.conn.lock().unwrap();
        let calls = conn
            .query_row(
                r#"
                SELECT COUNT(*)
                FROM tool_calls AS call
                JOIN turns AS turn ON turn.id = call.turn_id
                WHERE turn.strand_id = ?1
                "#,
                params![strand],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| error.to_string())?;
        let mut stmt = conn
            .prepare(
                r#"
                SELECT result.output, result.error_text
                FROM tool_results AS result
                JOIN tool_calls AS call ON call.id = result.tool_call_id
                JOIN turns AS turn ON turn.id = call.turn_id
                WHERE turn.strand_id = ?1
                "#,
            )
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map(params![strand], |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                ))
            })
            .map_err(|error| error.to_string())?;
        let mut held = 0usize;
        for row in rows {
            let (output, error) = row.map_err(|error| error.to_string())?;
            held = held.saturating_add(match output {
                Some(output) => captured(&output),
                None => error.as_deref().map_or(0, str::len),
            });
        }
        Ok(budget::Usage {
            calls: usize::try_from(calls).unwrap_or(usize::MAX),
            output: held,
        })
    }
}

fn captured(output: &str) -> usize {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(output) else {
        return output.len();
    };
    let stdout = value
        .get("stdout")
        .and_then(serde_json::Value::as_str)
        .map_or(0, str::len);
    let stderr = value
        .get("stderr")
        .and_then(serde_json::Value::as_str)
        .map_or(0, str::len);
    if value.get("stdout").is_some() || value.get("stderr").is_some() {
        stdout.saturating_add(stderr)
    } else {
        output.len()
    }
}
