mod state;

use rusqlite::params;
use santi_error::{
    ErrorIncident, ErrorScope, ErrorSource, IncidentDraft, SantiError, catalog, engine,
};
use santi_provider::{ProviderItem, ProviderTool};
use serde_json::{Value, json};

use super::{
    STRAND_INBOX_GATE, SantiStore, StartedTurn,
    assembly::assembly_input_in_conn,
    db::{drain_inbox_in_tx, turn_by_id},
    rows::message_kind_db,
};
use crate::{
    ContextEstimate, InboxSource, IngestOutcome, MessageContent, MessageKind, prefixed_id,
    timestamp_now,
};

use state::{current_strand_seq, open_context_incident, pending_items, repeat_context_incident};

const REASON_PENDING: &str = "pending_drain_would_exceed_budget";

pub(crate) fn context_incident_key(strand_id: &str) -> String {
    format!(
        "{}:strand:{strand_id}",
        catalog::CONTEXT_BUDGET_EXCEEDED.code
    )
}

pub(crate) struct ContextIncidentInput<'a> {
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

impl ContextIncidentInput<'_> {
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

pub(crate) struct ContextAdmission {
    pub provider: String,
    pub model: String,
    pub budget_source: String,
    pub budget_bytes: i64,
    pub instructions: Option<String>,
    pub tools: Vec<ProviderTool>,
}

impl SantiStore {
    pub(crate) fn enqueue_inbox_with_context(
        &self,
        strand_id: &str,
        message_kind: MessageKind,
        content: MessageContent,
        source: Option<InboxSource>,
        admission: Option<&ContextAdmission>,
    ) -> Result<IngestOutcome, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;

        if crate::store::errors::active_in_conn(&tx, &context_incident_key(strand_id))?.is_some() {
            let error = repeat_context_incident(&tx, strand_id, "ingest_active_guard")?;
            tx.commit().map_err(|error| error.to_string())?;
            return Ok(IngestOutcome::Rejected {
                error: Box::new(error),
            });
        }

        if let Some(admission) = admission {
            let mut input = assembly_input_in_conn(&tx, strand_id)?;
            input.extend(pending_items(&tx, strand_id)?);
            if let Some(candidate) =
                crate::context_budget::inbound_provider_item(&message_kind, &content)
            {
                input.push(candidate);
            }
            let estimate = crate::context_budget::estimate_provider_parts(
                &input,
                admission.instructions.as_deref(),
                Some(admission.tools.as_slice()),
            );
            if estimate.total_bytes > admission.budget_bytes {
                let reason = over_budget_reason(estimate.total_bytes, admission.budget_bytes);
                let observed_at_seq = current_strand_seq(&tx, strand_id)?;
                let error = open_context_incident(
                    &tx,
                    strand_id,
                    ContextIncidentInput {
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
                params![strand_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if pending >= STRAND_INBOX_GATE {
            let message =
                format!("strand inbox is full ({pending} pending, gate {STRAND_INBOX_GATE})");
            let error = engine().transient(
                catalog::INBOX_CAPACITY_EXCEEDED,
                ErrorSource::new("santi-core", "ingest_admission"),
                Some(ErrorScope::new("strand", strand_id)),
                message,
                json!({"pending": pending, "gate": STRAND_INBOX_GATE}),
            );
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
                strand_id,
                message_kind_db(&message_kind),
                content_json,
                source_type,
                source_ref,
                source_metadata,
                now
            ],
        )
        .map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(IngestOutcome::Accepted {
            strand_id: strand_id.to_string(),
        })
    }

    pub(crate) fn start_turn_with_budget(
        &self,
        strand_id: &str,
        trigger_type: &str,
        trigger_ref: Option<&str>,
        admission: Option<&ContextAdmission>,
    ) -> Result<Option<StartedTurn>, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let running: Option<i64> = tx
            .query_row(
                "SELECT 1 FROM turns WHERE strand_id = ?1 AND status = 'running' LIMIT 1",
                params![strand_id],
                |row| row.get(0),
            )
            .ok();
        if running.is_some() {
            return Ok(None);
        }

        let pending = pending_items(&tx, strand_id)?;
        if pending.is_empty() {
            return Ok(None);
        }

        if crate::store::errors::active_in_conn(&tx, &context_incident_key(strand_id))?.is_some() {
            repeat_context_incident(&tx, strand_id, "pending_active_guard")?;
            tx.commit().map_err(|error| error.to_string())?;
            return Ok(None);
        }

        if let Some(admission) = admission {
            let mut input = assembly_input_in_conn(&tx, strand_id)?;
            input.extend(pending);
            let estimate = crate::context_budget::estimate_provider_parts(
                &input,
                admission.instructions.as_deref(),
                Some(admission.tools.as_slice()),
            );
            if estimate.total_bytes > admission.budget_bytes {
                let reason = over_budget_reason(estimate.total_bytes, admission.budget_bytes);
                let observed_at_seq = current_strand_seq(&tx, strand_id)?;
                open_context_incident(
                    &tx,
                    strand_id,
                    ContextIncidentInput {
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
                return Ok(None);
            }
        }

        let turn_id = prefixed_id("turn");
        let drained_messages = drain_inbox_in_tx(&tx, strand_id, &turn_id)?;
        if drained_messages.is_empty() {
            return Ok(None);
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
            params![turn_id, strand_id, trigger_type, trigger_ref, now],
        )
        .map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(Some(StartedTurn {
            turn: turn_by_id(&conn, &turn_id)?.ok_or_else(|| "created turn missing".to_string())?,
            drained_messages,
        }))
    }

    pub(crate) fn pending_provider_items(
        &self,
        strand_id: &str,
    ) -> Result<Vec<ProviderItem>, String> {
        let conn = self.conn.lock().unwrap();
        pending_items(&conn, strand_id)
    }

    pub(crate) fn open_context_incident(
        &self,
        strand_id: &str,
        input: ContextIncidentInput<'_>,
    ) -> Result<SantiError, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let error = open_context_incident(&tx, strand_id, input)?;
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
        let resolved = crate::store::errors::resolve_in_conn(
            &tx,
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
