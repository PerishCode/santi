use rusqlite::{Connection, params};
use santi_model::prefixed_id;

use super::*;

#[derive(Debug)]
struct V23Block {
    id: String,
    strand_id: String,
    status: String,
    reason_code: String,
    reason_text: String,
    provider: Option<String>,
    model: Option<String>,
    budget_source: Option<String>,
    budget_bytes: Option<i64>,
    input_items: Option<i64>,
    input_bytes: Option<i64>,
    instructions_bytes: Option<i64>,
    tools_bytes: Option<i64>,
    total_bytes: Option<i64>,
    observed_turn_id: Option<String>,
    observed_at_seq: Option<i64>,
    metadata: Option<String>,
    created_at: String,
    updated_at: String,
    cleared_at: Option<String>,
    cleared_by: Option<String>,
}

pub fn migrate_v23_to_v24(conn: &Connection) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    tx.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS error_incidents (
            id TEXT PRIMARY KEY,
            incident_key TEXT NOT NULL,
            code TEXT NOT NULL,
            status TEXT NOT NULL CHECK (status IN ('active', 'resolved')),
            category TEXT NOT NULL,
            severity TEXT NOT NULL,
            retry TEXT NOT NULL,
            exposure TEXT NOT NULL,
            scope_kind TEXT NOT NULL,
            scope_id TEXT NOT NULL,
            source_component TEXT NOT NULL,
            source_operation TEXT NOT NULL,
            latest_source_component TEXT NOT NULL,
            latest_source_operation TEXT NOT NULL,
            message TEXT NOT NULL,
            latest_message TEXT NOT NULL,
            context TEXT NOT NULL,
            latest_context TEXT NOT NULL,
            occurrence_count INTEGER NOT NULL CHECK (occurrence_count > 0),
            revision INTEGER NOT NULL CHECK (revision > 0),
            first_seen_at TEXT NOT NULL,
            last_seen_at TEXT NOT NULL,
            resolved_at TEXT,
            resolved_by TEXT
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_error_incidents_active_key
        ON error_incidents(incident_key)
        WHERE status = 'active';
        CREATE INDEX IF NOT EXISTS idx_error_incidents_scope_time
        ON error_incidents(scope_kind, scope_id, first_seen_at);

        CREATE TABLE IF NOT EXISTS error_transitions (
            id TEXT PRIMARY KEY,
            incident_id TEXT NOT NULL,
            revision INTEGER NOT NULL,
            kind TEXT NOT NULL CHECK (kind IN ('opened', 'resolved')),
            payload TEXT NOT NULL,
            created_at TEXT NOT NULL,
            delivered_at TEXT,
            UNIQUE (incident_id, revision)
        );
        CREATE INDEX IF NOT EXISTS idx_error_transitions_pending
        ON error_transitions(created_at, id)
        WHERE delivered_at IS NULL;
        "#,
    )
    .map_err(|error| error.to_string())?;

    let schema = Schema(&tx);
    if schema.table_exists("strand_blocks")? {
        let mut stmt = tx
            .prepare(
                r#"
                SELECT id, strand_id, status, reason_code, reason_text, provider,
                       model, budget_source, budget_bytes, input_items, input_bytes,
                       instructions_bytes, tools_bytes, total_bytes, observed_turn_id,
                       observed_at_seq, metadata, created_at, updated_at, cleared_at,
                       cleared_by
                FROM strand_blocks
                ORDER BY created_at ASC, id ASC
                "#,
            )
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok(V23Block {
                    id: row.get(0)?,
                    strand_id: row.get(1)?,
                    status: row.get(2)?,
                    reason_code: row.get(3)?,
                    reason_text: row.get(4)?,
                    provider: row.get(5)?,
                    model: row.get(6)?,
                    budget_source: row.get(7)?,
                    budget_bytes: row.get(8)?,
                    input_items: row.get(9)?,
                    input_bytes: row.get(10)?,
                    instructions_bytes: row.get(11)?,
                    tools_bytes: row.get(12)?,
                    total_bytes: row.get(13)?,
                    observed_turn_id: row.get(14)?,
                    observed_at_seq: row.get(15)?,
                    metadata: row.get(16)?,
                    created_at: row.get(17)?,
                    updated_at: row.get(18)?,
                    cleared_at: row.get(19)?,
                    cleared_by: row.get(20)?,
                })
            })
            .map_err(|error| error.to_string())?;
        let mut blocks = Vec::new();
        for row in rows {
            blocks.push(row.map_err(|error| error.to_string())?);
        }
        drop(stmt);

        let has_rejections = schema.table_exists("rejected_deliveries")?;
        for block in blocks {
            let occurrences = if has_rejections {
                tx.query_row(
                    "SELECT COUNT(*) FROM rejected_deliveries WHERE block_id = ?1",
                    params![block.id],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|error| error.to_string())?
                .max(1)
            } else {
                1
            };
            let metadata = block
                .metadata
                .as_deref()
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok());
            let operation = metadata
                .as_ref()
                .and_then(|value| value.get("phase"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or(&block.reason_code)
                .to_string();
            let budget = serde_json::json!({
                "source": block.budget_source,
                "input_bytes": block.budget_bytes,
            });
            let estimate = serde_json::json!({
                "input_items": block.input_items,
                "input_bytes": block.input_bytes,
                "instructions_bytes": block.instructions_bytes,
                "tools_bytes": block.tools_bytes,
                "total_bytes": block.total_bytes,
            });
            let context = serde_json::json!({
                "schema": "santi.error.context_budget.v1",
                "reason": block.reason_code,
                "provider": block.provider,
                "model": block.model,
                "budget": budget,
                "estimate": estimate,
                "observed_turn_id": block.observed_turn_id,
                "observed_at_seq": block.observed_at_seq,
                "details": metadata,
                "migration": "v23_to_v24",
            });
            let status = if block.status == "active" {
                "active"
            } else {
                "resolved"
            };
            let revision = if status == "active" { 1 } else { 2 };
            let incident_id = block
                .id
                .strip_prefix("blk_")
                .map(|suffix| format!("inc_{suffix}"))
                .unwrap_or_else(|| prefixed_id("inc"));
            tx.execute(
                r#"
                INSERT INTO error_incidents (
                  id, incident_key, code, status, category, severity, retry, exposure,
                  scope_kind, scope_id, source_component, source_operation,
                  latest_source_component, latest_source_operation, message,
                  latest_message, context, latest_context, occurrence_count, revision,
                  first_seen_at, last_seen_at, resolved_at, resolved_by
                ) VALUES (
                  ?1, ?2, 'context.budget.exceeded', ?3, 'resource_exhausted',
                  'error', 'after_resolution', ?4, 'strand', ?5, 'santi-core', ?6,
                  'santi-core', ?6, ?7, ?7, ?8, ?8, ?9, ?10, ?11, ?12, ?13, ?14
                )
                "#,
                params![
                    incident_id,
                    format!("context.budget.exceeded:strand:{}", block.strand_id),
                    status,
                    serde_json::to_string(&santi_error::ErrorExposure::CALLER_AND_OPERATOR)
                        .map_err(|error| error.to_string())?,
                    block.strand_id,
                    operation,
                    block.reason_text,
                    serde_json::to_string(&context).map_err(|error| error.to_string())?,
                    occurrences,
                    revision,
                    block.created_at,
                    block.updated_at,
                    block.cleared_at,
                    block.cleared_by,
                ],
            )
            .map_err(|error| error.to_string())?;
        }
    }

    tx.execute_batch(
        r#"
        DROP TABLE IF EXISTS rejected_deliveries;
        DROP TABLE IF EXISTS strand_blocks;
        "#,
    )
    .map_err(|error| error.to_string())?;
    tx.commit().map_err(|error| error.to_string())
}
