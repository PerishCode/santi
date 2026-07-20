use super::*;

impl SantiStore {
    pub(crate) fn start_turn_with_budget(
        &self,
        launch: Launch<'_>,
    ) -> Result<StartTurnOutcome, String> {
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
        let running_id: Option<String> = tx
            .query_row(
                "SELECT id FROM turns WHERE strand_id = ?1 AND status = 'running' LIMIT 1",
                params![strand],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        if let Some(turn_id) = running_id {
            let turn = Database::new(&tx)
                .turn_by_id(&turn_id)?
                .ok_or_else(|| "running turn missing".to_string())?;
            return Ok(StartTurnOutcome::Running(turn));
        }

        let database = Database::new(&tx);
        let pending = database.pending_items(strand)?;
        let has_failed_receipt = recover
            && tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM inbox_receipts WHERE strand_id = ?1 AND state = 'turn_failed')",
                    params![strand],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|error| error.to_string())?
                != 0;
        if pending.is_empty() && !has_failed_receipt {
            return Ok(StartTurnOutcome::Idle);
        }

        if database
            .active_incident(&context_incident_key(strand))?
            .is_some()
        {
            let error = database.repeat_context_incident(strand, "pending_active_guard")?;
            tx.commit().map_err(|error| error.to_string())?;
            return Ok(StartTurnOutcome::Held(error));
        }

        if let Some(admission) = admission {
            let mut input = assembly_input_in_conn(&tx, strand)?;
            input.extend(pending);
            let estimate = crate::context::budget::estimate_provider_parts(
                &input,
                admission.instructions.as_deref(),
                Some(admission.tools.as_slice()),
            );
            if estimate.total_bytes > admission.budget_bytes {
                let reason = over_budget_reason(estimate.total_bytes, admission.budget_bytes);
                let observed_at_seq = database.current_strand_seq(strand)?;
                let error = database.open_context_incident(
                    strand,
                    Pressure {
                        reason_code: REASON_PENDING,
                        reason_text: &reason,
                        operation: "pending_drain_admission",
                        provider: Some(&admission.provider),
                        model: Some(&admission.model),
                        budget_source: Some(&admission.budget_source),
                        budget_bytes: Some(admission.budget_bytes),
                        estimate: &estimate,
                        observed_turn_id: None,
                        observed_at_seq,
                        metadata: Some(json!({"estimator": estimate.estimator})),
                    },
                )?;
                tx.commit().map_err(|error| error.to_string())?;
                return Ok(StartTurnOutcome::Held(error));
            }
        }

        let turn_id = prefixed_id("turn");
        let drained = drain_inbox_in_tx(&tx, strand, &turn_id)?;
        if drained.messages.is_empty() && !has_failed_receipt {
            return Ok(StartTurnOutcome::Idle);
        }
        let now = timestamp_now();
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
            params![turn_id, strand, trigger, reference, now],
        )
        .map_err(|error| error.to_string())?;
        let recovered_incident_id = crate::store::errors::drive::resolve_in_conn(
            &tx,
            strand,
            &turn_id,
            drained.messages.len(),
        )?;
        Database::new(&tx).begin_turn(
            strand,
            &turn_id,
            &drained.inbox_ids,
            recovered_incident_id.as_deref(),
        )?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(StartTurnOutcome::Started(StartedTurn {
            turn: Database::new(&conn)
                .turn_by_id(&turn_id)?
                .ok_or_else(|| "created turn missing".to_string())?,
            drained_messages: drained.messages,
        }))
    }
}

pub(crate) fn execution_budget_incident_key(strand_id: &str) -> String {
    format!(
        "{}:strand:{strand_id}",
        catalog::EXECUTION_BUDGET_EXCEEDED.code
    )
}

impl SantiStore {
    pub(crate) fn strand_execution_usage(&self, strand_id: &str) -> Result<Usage, String> {
        let conn = self.conn.lock().unwrap();
        let tool_calls = conn
            .query_row(
                r#"
                SELECT COUNT(*)
                FROM tool_calls AS call
                JOIN turns AS turn ON turn.id = call.turn_id
                WHERE turn.strand_id = ?1
                "#,
                params![strand_id],
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
            .query_map(params![strand_id], |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                ))
            })
            .map_err(|error| error.to_string())?;
        let mut tool_output_bytes = 0usize;
        for row in rows {
            let (output, error_text) = row.map_err(|error| error.to_string())?;
            tool_output_bytes = tool_output_bytes.saturating_add(match output {
                Some(output) => captured_output_bytes(&output),
                None => error_text.as_deref().map_or(0, str::len),
            });
        }
        Ok(Usage {
            tool_calls: usize::try_from(tool_calls).unwrap_or(usize::MAX),
            tool_output_bytes,
        })
    }
}

fn captured_output_bytes(output: &str) -> usize {
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
