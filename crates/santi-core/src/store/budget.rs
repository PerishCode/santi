mod state;

use rusqlite::{OptionalExtension, params};
use santi_error::{
    ErrorIncident, ErrorScope, ErrorSource, IncidentDraft, SantiError, catalog, engine,
};
use santi_provider::{ProviderItem, ProviderTool};
use serde_json::{Value, json};

use super::{
    STRAND_INBOX_GATE, SantiStore, StartTurnOutcome, StartedTurn,
    assembly::assembly_input_in_conn,
    db::{Database, drain_inbox_in_tx},
};
use crate::{
    ContextEstimate, InboxSource, IngestOutcome, IngestReceipt, MessageContent, MessageKind, Usage,
    prefixed_id, timestamp_now,
};

const REASON_PENDING: &str = "pending_drain_would_exceed_budget";

pub(crate) fn context_incident_key(strand_id: &str) -> String {
    format!(
        "{}:strand:{strand_id}",
        catalog::CONTEXT_BUDGET_EXCEEDED.code
    )
}

pub(crate) struct Pressure<'a> {
    pub reason_code: &'a str,
    pub reason_text: &'a str,
    pub operation: &'a str,
    pub provider: Option<&'a str>,
    pub model: Option<&'a str>,
    pub budget_source: Option<&'a str>,
    pub budget_bytes: Option<i64>,
    pub estimate: &'a ContextEstimate,
    pub observed_turn_id: Option<&'a str>,
    pub observed_at_seq: Option<i64>,
    pub metadata: Option<Value>,
}

impl Pressure<'_> {
    fn into_draft(self, strand_id: &str) -> IncidentDraft {
        IncidentDraft {
            incident_key: context_incident_key(strand_id),
            descriptor: catalog::CONTEXT_BUDGET_EXCEEDED,
            scope: ErrorScope::new("strand", strand_id),
            source: ErrorSource::new("santi-core", self.operation),
            message: self.reason_text.to_string(),
            context: json!({
                "schema": "santi.error.context_budget.v1",
                "reason": self.reason_code,
                "provider": self.provider,
                "model": self.model,
                "budget": {
                    "source": self.budget_source,
                    "input_bytes": self.budget_bytes,
                },
                "estimate": self.estimate,
                "observed_turn_id": self.observed_turn_id,
                "observed_at_seq": self.observed_at_seq,
                "details": self.metadata,
            }),
        }
    }
}

pub(crate) struct Admission {
    pub provider: String,
    pub model: String,
    pub budget_source: String,
    pub budget_bytes: i64,
    pub instructions: Option<String>,
    pub tools: Vec<ProviderTool>,
}

pub(crate) struct Ingress<'a> {
    pub strand: &'a str,
    pub kind: MessageKind,
    pub content: MessageContent,
    pub source: Option<InboxSource>,
    pub admission: Option<&'a Admission>,
}

pub(crate) struct Launch<'a> {
    pub strand: &'a str,
    pub trigger: &'a str,
    pub reference: Option<&'a str>,
    pub admission: Option<&'a Admission>,
    pub recover: bool,
}

impl SantiStore {
    pub(crate) fn enqueue_inbox_with_context(
        &self,
        ingress: Ingress<'_>,
    ) -> Result<IngestOutcome, String> {
        self.enqueue_inbox_with_policy(ingress, true)
    }

    pub(crate) fn enqueue_inbox_while_suspended(
        &self,
        strand_id: &str,
        message_kind: MessageKind,
        content: MessageContent,
        source: Option<InboxSource>,
    ) -> Result<IngestOutcome, String> {
        self.enqueue_inbox_with_policy(
            Ingress {
                strand: strand_id,
                kind: message_kind,
                content,
                source,
                admission: None,
            },
            false,
        )
    }

    fn enqueue_inbox_with_policy(
        &self,
        ingress: Ingress<'_>,
        enforce_active_holds: bool,
    ) -> Result<IngestOutcome, String> {
        let Ingress {
            strand,
            kind,
            content,
            source,
            admission,
        } = ingress;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;

        if enforce_active_holds
            && let Some(error) =
                super::errors::drive::repeat_active_in_conn(&tx, strand, "ingest_active_guard")?
        {
            tx.commit().map_err(|error| error.to_string())?;
            return Ok(IngestOutcome::Rejected {
                error: Box::new(error),
            });
        }

        if enforce_active_holds
            && Database::new(&tx)
                .active_incident(&context_incident_key(strand))?
                .is_some()
        {
            let error =
                Database::new(&tx).repeat_context_incident(strand, "ingest_active_guard")?;
            tx.commit().map_err(|error| error.to_string())?;
            return Ok(IngestOutcome::Rejected {
                error: Box::new(error),
            });
        }

        if let Some(admission) = admission {
            let database = Database::new(&tx);
            let mut input = assembly_input_in_conn(&tx, strand)?;
            input.extend(database.pending_items(strand)?);
            if let Some(candidate) = crate::context::budget::inbound_provider_item(&kind, &content)
            {
                input.push(candidate);
            }
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
                        reason_code: "candidate_input_exceeds_budget",
                        reason_text: &reason,
                        operation: "ingest_admission",
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
                return Ok(IngestOutcome::Rejected {
                    error: Box::new(error),
                });
            }
        }

        let pending: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM strand_inbox WHERE strand_id = ?1",
                params![strand],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if pending >= STRAND_INBOX_GATE {
            let message =
                format!("strand inbox is full ({pending} pending, gate {STRAND_INBOX_GATE})");
            let error = engine().transient(crate::Signal {
                descriptor: catalog::INBOX_CAPACITY_EXCEEDED,
                source: ErrorSource::new("santi-core", "ingest_admission"),
                scope: Some(ErrorScope::new("strand", strand)),
                message,
                context: json!({"pending": pending, "gate": STRAND_INBOX_GATE}),
            });
            return Ok(IngestOutcome::Rejected {
                error: Box::new(error),
            });
        }

        let inbox_id = prefixed_id("inbox");
        let now = timestamp_now();
        let content_json = serde_json::to_string(&content).map_err(|error| error.to_string())?;
        let source_type = source.as_ref().map(|source| source.source_type.as_str());
        let source_ref = source
            .as_ref()
            .and_then(|source| source.source_ref.as_deref());
        let source_metadata = source
            .as_ref()
            .and_then(|source| source.metadata.as_ref())
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| error.to_string())?;
        tx.execute(
            r#"
            INSERT INTO strand_inbox (
              id, strand_id, message_kind, content, source_type, source_ref, source_metadata, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            params![
                inbox_id,
                strand,
                kind.encode(),
                content_json,
                source_type,
                source_ref,
                source_metadata,
                now
            ],
        )
        .map_err(|error| error.to_string())?;
        Database::new(&tx).insert_accepted(&inbox_id, strand, &now)?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(IngestOutcome::Accepted {
            receipt: IngestReceipt {
                strand_id: strand.to_string(),
                inbox_id,
                warning: None,
            },
        })
    }

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
        let recovered_incident_id =
            super::errors::drive::resolve_in_conn(&tx, strand, &turn_id, drained.messages.len())?;
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

    pub(crate) fn pending_provider_items(
        &self,
        strand_id: &str,
    ) -> Result<Vec<ProviderItem>, String> {
        let conn = self.conn.lock().unwrap();
        Database::new(&conn).pending_items(strand_id)
    }

    pub(crate) fn open_context_incident(
        &self,
        strand_id: &str,
        input: Pressure<'_>,
    ) -> Result<SantiError, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let error = Database::new(&tx).open_context_incident(strand_id, input)?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(error)
    }

    pub(crate) fn active_context_incident(
        &self,
        strand_id: &str,
    ) -> Result<Option<ErrorIncident>, String> {
        self.active_error_incident(&context_incident_key(strand_id))
    }

    pub(crate) fn resolve_context_incident(
        &self,
        strand_id: &str,
        resolved_by: &str,
        estimate: &ContextEstimate,
    ) -> Result<bool, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let resolved = Database::new(&tx).resolve_incident(
            &context_incident_key(strand_id),
            resolved_by,
            json!({
                "schema": "santi.error.context_budget.resolution.v1",
                "resolved_by": resolved_by,
                "estimate": estimate,
            }),
        )?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(resolved)
    }
}

fn over_budget_reason(total_bytes: i64, budget_bytes: i64) -> String {
    format!("strand context is over budget ({total_bytes} estimated bytes, budget {budget_bytes})")
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
