mod state;

use rusqlite::params;
use santi_provider::{ProviderItem, ProviderTool};
use serde_json::Value;

use super::{
    STRAND_INBOX_GATE, SantiStore, StartedTurn,
    assembly::assembly_input_in_conn,
    db::{drain_inbox_in_tx, turn_by_id},
    rows::message_kind_db,
};
use crate::{
    ContextEstimate, InboxSource, IngestOutcome, MessageContent, MessageKind, RejectedDelivery,
    StrandBlock, prefixed_id, timestamp_now,
};

use state::{
    active_block, blocked_reason, current_strand_seq, insert_rejection, pending_items,
    reject_pending_inbox, upsert_block,
};

pub(super) use state::{rejected_deliveries_for_strand, strand_blocks_for_strand};

const REASON_ACTIVE_BLOCK: &str = "context_over_budget_active";
const REASON_PENDING: &str = "pending_drain_would_exceed_budget";

pub(crate) struct ContextBlockInput<'a> {
    pub reason_code: &'a str,
    pub reason_text: &'a str,
    pub provider: &'a str,
    pub model: &'a str,
    pub budget_source: Option<&'a str>,
    pub budget_bytes: Option<i64>,
    pub estimate: &'a ContextEstimate,
    pub observed_turn_id: Option<&'a str>,
    pub observed_at_seq: Option<i64>,
    pub metadata: Option<Value>,
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
        let tx = conn.transaction().map_err(|error| error.to_string())?;

        if let Some(block) = active_block(&tx, strand_id)? {
            let reason = blocked_reason(&block.id, &block.reason_text);
            insert_rejection(
                &tx,
                Some(strand_id),
                Some(&block.id),
                source.as_ref(),
                Some(&message_kind),
                &content,
                "context_over_budget_active",
                &reason,
            )?;
            tx.commit().map_err(|error| error.to_string())?;
            return Ok(IngestOutcome::Rejected { reason });
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
                let reason = format!(
                    "strand context is over budget ({} estimated bytes, budget {})",
                    estimate.total_bytes, admission.budget_bytes
                );
                let block = upsert_block(
                    &tx,
                    strand_id,
                    ContextBlockInput {
                        reason_code: "candidate_input_exceeds_budget",
                        reason_text: &reason,
                        provider: &admission.provider,
                        model: &admission.model,
                        budget_source: Some(&admission.budget_source),
                        budget_bytes: Some(admission.budget_bytes),
                        estimate: &estimate,
                        observed_turn_id: None,
                        observed_at_seq: current_strand_seq(&tx, strand_id)?,
                        metadata: Some(serde_json::json!({
                            "phase": "ingest_admission",
                            "estimator": estimate.estimator,
                        })),
                    },
                )?;
                insert_rejection(
                    &tx,
                    Some(strand_id),
                    Some(&block.id),
                    source.as_ref(),
                    Some(&message_kind),
                    &content,
                    "candidate_input_exceeds_budget",
                    &reason,
                )?;
                tx.commit().map_err(|error| error.to_string())?;
                return Ok(IngestOutcome::Rejected { reason });
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
            let reason =
                format!("strand inbox is full ({pending} pending, gate {STRAND_INBOX_GATE})");
            insert_rejection(
                &tx,
                Some(strand_id),
                None,
                source.as_ref(),
                Some(&message_kind),
                &content,
                "strand_inbox_full",
                &reason,
            )?;
            tx.commit().map_err(|error| error.to_string())?;
            return Ok(IngestOutcome::Rejected { reason });
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
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
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
        let tx = conn.transaction().map_err(|error| error.to_string())?;
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

        if let Some(block) = active_block(&tx, strand_id)? {
            let reason = blocked_reason(&block.id, &block.reason_text);
            reject_pending_inbox(&tx, strand_id, &block.id, REASON_ACTIVE_BLOCK, &reason)?;
            tx.commit().map_err(|error| error.to_string())?;
            return Ok(None);
        }

        let pending = pending_items(&tx, strand_id)?;
        if pending.is_empty() {
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
                let reason = format!(
                    "strand context is over budget ({} estimated bytes, budget {})",
                    estimate.total_bytes, admission.budget_bytes
                );
                let block = upsert_block(
                    &tx,
                    strand_id,
                    ContextBlockInput {
                        reason_code: REASON_PENDING,
                        reason_text: &reason,
                        provider: &admission.provider,
                        model: &admission.model,
                        budget_source: Some(&admission.budget_source),
                        budget_bytes: Some(admission.budget_bytes),
                        estimate: &estimate,
                        observed_turn_id: None,
                        observed_at_seq: current_strand_seq(&tx, strand_id)?,
                        metadata: Some(serde_json::json!({
                            "phase": "pending_drain_admission",
                            "estimator": estimate.estimator,
                        })),
                    },
                )?;
                reject_pending_inbox(&tx, strand_id, &block.id, REASON_PENDING, &reason)?;
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

    pub(crate) fn active_context_block(
        &self,
        strand_id: &str,
    ) -> Result<Option<StrandBlock>, String> {
        let conn = self.conn.lock().unwrap();
        active_block(&conn, strand_id)
    }

    pub(crate) fn rejected_deliveries(
        &self,
        strand_id: &str,
        limit: i64,
    ) -> Result<Vec<RejectedDelivery>, String> {
        let conn = self.conn.lock().unwrap();
        rejected_deliveries_for_strand(&conn, strand_id, limit)
    }

    pub(crate) fn pending_provider_items(
        &self,
        strand_id: &str,
    ) -> Result<Vec<ProviderItem>, String> {
        let conn = self.conn.lock().unwrap();
        pending_items(&conn, strand_id)
    }

    pub(crate) fn upsert_context_block(
        &self,
        strand_id: &str,
        input: ContextBlockInput<'_>,
    ) -> Result<StrandBlock, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        upsert_block(&tx, strand_id, input)?;
        tx.commit().map_err(|error| error.to_string())?;
        drop(conn);
        self.active_context_block(strand_id)?
            .ok_or_else(|| "context block missing after upsert".to_string())
    }

    pub(crate) fn clear_active_context_block(
        &self,
        strand_id: &str,
        cleared_by: &str,
        estimate: &ContextEstimate,
    ) -> Result<bool, String> {
        let conn = self.conn.lock().unwrap();
        let Some(block) = active_block(&conn, strand_id)? else {
            return Ok(false);
        };
        let now = timestamp_now();
        let metadata = serde_json::to_string(&serde_json::json!({
            "schema": "santi.context_block_clear.v1",
            "cleared_by": cleared_by,
            "clear_estimate": estimate,
            "previous_metadata": block.metadata,
        }))
        .map_err(|error| error.to_string())?;
        conn.execute(
            r#"
            UPDATE strand_blocks
            SET status = 'cleared', updated_at = ?2, cleared_at = ?2,
                cleared_by = ?3, metadata = ?4
            WHERE id = ?1 AND status = 'active'
            "#,
            params![block.id, now, cleared_by, metadata],
        )
        .map_err(|error| error.to_string())?;
        Ok(true)
    }
}
