use rusqlite::{Connection, OptionalExtension, params};
use santi_provider::ProviderItem;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::ContextBlockInput;
use crate::{
    InboxSource, MessageContent, MessageKind, RejectedDelivery, StrandBlock, prefixed_id,
    timestamp_now,
};

use crate::store::rows::{
    collect_rows, map_rejected_delivery_row, map_strand_block_row, message_kind_db,
    message_kind_from_db,
};

const REJECT_EXCERPT_BYTES: usize = 1024;
const REJECT_EXCERPT_TRUNCATED: &str = "\n[truncated]";
const REJECT_SOURCE_METADATA_BYTES: usize = 4096;

pub(super) fn reject_pending_inbox(
    conn: &Connection,
    strand_id: &str,
    block_id: &str,
    reason_code: &str,
    reason_text: &str,
) -> Result<usize, String> {
    let mut stmt = conn
        .prepare(
            r#"
            SELECT id, message_kind, content, source_type, source_ref, source_metadata
            FROM strand_inbox
            WHERE strand_id = ?1
            ORDER BY rowid ASC
            "#,
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(params![strand_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })
        .map_err(|error| error.to_string())?;
    let mut pending = Vec::new();
    for row in rows {
        pending.push(row.map_err(|error| error.to_string())?);
    }
    drop(stmt);

    let rejected = pending.len();
    for (id, kind, content_json, source_type, source_ref, source_metadata) in pending {
        let content = serde_json::from_str::<MessageContent>(&content_json)
            .map_err(|error| error.to_string())?;
        let metadata = source_metadata.and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
        let source = source_type.map(|source_type| InboxSource {
            source_type,
            source_ref,
            metadata,
        });
        let message_kind = message_kind_from_db(&kind);
        insert_rejection(
            conn,
            Some(strand_id),
            Some(block_id),
            source.as_ref(),
            Some(&message_kind),
            &content,
            reason_code,
            reason_text,
        )?;
        conn.execute("DELETE FROM strand_inbox WHERE id = ?1", params![id])
            .map_err(|error| error.to_string())?;
    }
    Ok(rejected)
}

pub(super) fn pending_items(
    conn: &Connection,
    strand_id: &str,
) -> Result<Vec<ProviderItem>, String> {
    let mut stmt = conn
        .prepare(
            r#"
            SELECT message_kind, content
            FROM strand_inbox
            WHERE strand_id = ?1
            ORDER BY rowid ASC
            "#,
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(params![strand_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| error.to_string())?;
    let mut items = Vec::new();
    for row in rows {
        let (kind, content_json) = row.map_err(|error| error.to_string())?;
        let content = serde_json::from_str::<MessageContent>(&content_json)
            .map_err(|error| error.to_string())?;
        if let Some(item) =
            crate::context_budget::inbound_provider_item(&message_kind_from_db(&kind), &content)
        {
            items.push(item);
        }
    }
    Ok(items)
}

pub(super) fn upsert_block(
    conn: &Connection,
    strand_id: &str,
    input: ContextBlockInput<'_>,
) -> Result<StrandBlock, String> {
    let now = timestamp_now();
    let metadata = input
        .metadata
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|error| error.to_string())?;
    if let Some(existing) = active_block(conn, strand_id)? {
        conn.execute(
            r#"
            UPDATE strand_blocks
            SET reason_code = ?2, reason_text = ?3, provider = ?4, model = ?5,
                budget_source = ?6, budget_bytes = ?7, input_items = ?8,
                input_bytes = ?9, instructions_bytes = ?10, tools_bytes = ?11,
                total_bytes = ?12, observed_turn_id = ?13, observed_at_seq = ?14,
                metadata = ?15, updated_at = ?16
            WHERE id = ?1
            "#,
            params![
                existing.id,
                input.reason_code,
                input.reason_text,
                input.provider,
                input.model,
                input.budget_source,
                input.budget_bytes,
                input.estimate.input_items,
                input.estimate.input_bytes,
                input.estimate.instructions_bytes,
                input.estimate.tools_bytes,
                input.estimate.total_bytes,
                input.observed_turn_id,
                input.observed_at_seq,
                metadata,
                now,
            ],
        )
        .map_err(|error| error.to_string())?;
    } else {
        conn.execute(
            r#"
            INSERT INTO strand_blocks (
              id, strand_id, kind, status, reason_code, reason_text, provider, model,
              budget_source, budget_bytes, input_items, input_bytes, instructions_bytes,
              tools_bytes, total_bytes, observed_turn_id, observed_at_seq, metadata,
              created_at, updated_at, cleared_at, cleared_by
            )
            VALUES (
              ?1, ?2, 'context_over_budget', 'active', ?3, ?4, ?5, ?6,
              ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?17, NULL, NULL
            )
            "#,
            params![
                prefixed_id("blk"),
                strand_id,
                input.reason_code,
                input.reason_text,
                input.provider,
                input.model,
                input.budget_source,
                input.budget_bytes,
                input.estimate.input_items,
                input.estimate.input_bytes,
                input.estimate.instructions_bytes,
                input.estimate.tools_bytes,
                input.estimate.total_bytes,
                input.observed_turn_id,
                input.observed_at_seq,
                metadata,
                now,
            ],
        )
        .map_err(|error| error.to_string())?;
    }
    active_block(conn, strand_id)?.ok_or_else(|| "context block missing after upsert".to_string())
}

pub(super) fn current_strand_seq(
    conn: &Connection,
    strand_id: &str,
) -> Result<Option<i64>, String> {
    conn.query_row(
        "SELECT next_seq - 1 FROM strands WHERE id = ?1 LIMIT 1",
        params![strand_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(|error| error.to_string())
}

pub(super) fn blocked_reason(block_id: &str, reason_text: &str) -> String {
    format!("strand is blocked: context_over_budget block_id={block_id}: {reason_text}")
}

pub(in crate::store) fn strand_blocks_for_strand(
    conn: &Connection,
    strand_id: &str,
) -> Result<Vec<StrandBlock>, String> {
    let mut stmt = conn
        .prepare(
            r#"
            SELECT id, strand_id, kind, status, reason_code, reason_text,
                   provider, model, budget_source, budget_bytes, input_items,
                   input_bytes, instructions_bytes, tools_bytes, total_bytes,
                   observed_turn_id, observed_at_seq, metadata, created_at,
                   updated_at, cleared_at, cleared_by
            FROM strand_blocks
            WHERE strand_id = ?1
            ORDER BY created_at ASC, id ASC
            "#,
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(params![strand_id], map_strand_block_row)
        .map_err(|error| error.to_string())?;
    collect_rows(rows)
}

pub(in crate::store) fn rejected_deliveries_for_strand(
    conn: &Connection,
    strand_id: &str,
    limit: i64,
) -> Result<Vec<RejectedDelivery>, String> {
    let limit = limit.clamp(1, 1000);
    let mut stmt = conn
        .prepare(
            r#"
            SELECT id, strand_id, block_id, source_type, source_ref, source_metadata,
                   message_kind, content_sha256, content_bytes, content_excerpt,
                   reason_code, reason_text, received_at
            FROM rejected_deliveries
            WHERE strand_id = ?1
            ORDER BY received_at DESC, id DESC
            LIMIT ?2
            "#,
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(params![strand_id, limit], map_rejected_delivery_row)
        .map_err(|error| error.to_string())?;
    collect_rows(rows)
}

pub(super) fn active_block(
    conn: &Connection,
    strand_id: &str,
) -> Result<Option<StrandBlock>, String> {
    conn.query_row(
        r#"
        SELECT id, strand_id, kind, status, reason_code, reason_text,
               provider, model, budget_source, budget_bytes, input_items,
               input_bytes, instructions_bytes, tools_bytes, total_bytes,
               observed_turn_id, observed_at_seq, metadata, created_at,
               updated_at, cleared_at, cleared_by
        FROM strand_blocks
        WHERE strand_id = ?1 AND kind = 'context_over_budget' AND status = 'active'
        LIMIT 1
        "#,
        params![strand_id],
        map_strand_block_row,
    )
    .optional()
    .map_err(|error| error.to_string())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn insert_rejection(
    conn: &Connection,
    strand_id: Option<&str>,
    block_id: Option<&str>,
    source: Option<&InboxSource>,
    message_kind: Option<&MessageKind>,
    content: &MessageContent,
    reason_code: &str,
    reason_text: &str,
) -> Result<String, String> {
    let id = prefixed_id("reject");
    let content_json = serde_json::to_string(content).map_err(|error| error.to_string())?;
    let content_sha256 = content_sha256(&content_json);
    let content_excerpt = content_excerpt(content, &content_json);
    let source_metadata = source_metadata_json(source)?;
    let now = timestamp_now();
    conn.execute(
        r#"
        INSERT INTO rejected_deliveries (
          id, strand_id, block_id, source_type, source_ref, source_metadata,
          message_kind, content_sha256, content_bytes, content_excerpt,
          reason_code, reason_text, received_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#,
        params![
            id,
            strand_id,
            block_id,
            source.map(|source| source.source_type.as_str()),
            source.and_then(|source| source.source_ref.as_deref()),
            source_metadata,
            message_kind.map(message_kind_db),
            content_sha256,
            content_json.len() as i64,
            content_excerpt,
            reason_code,
            reason_text,
            now,
        ],
    )
    .map_err(|error| error.to_string())?;
    Ok(id)
}

fn content_sha256(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hex::encode(hasher.finalize())
}

fn content_excerpt(content: &MessageContent, fallback_json: &str) -> String {
    let text = content.content_text();
    let source = if text.trim().is_empty() {
        fallback_json
    } else {
        text.as_str()
    };
    cap_utf8_with_suffix(source, REJECT_EXCERPT_BYTES, REJECT_EXCERPT_TRUNCATED)
}

fn source_metadata_json(source: Option<&InboxSource>) -> Result<Option<String>, String> {
    let Some(metadata) = source.and_then(|source| source.metadata.as_ref()) else {
        return Ok(None);
    };
    let raw = serde_json::to_string(metadata).map_err(|error| error.to_string())?;
    if raw.len() <= REJECT_SOURCE_METADATA_BYTES {
        return Ok(Some(raw));
    }
    serde_json::to_string(&serde_json::json!({
        "schema": "santi.rejected_source_metadata_truncated.v1",
        "truncated": true,
        "original_bytes": raw.len(),
        "original_sha256": content_sha256(&raw),
    }))
    .map(Some)
    .map_err(|error| error.to_string())
}

fn cap_utf8_with_suffix(source: &str, max_bytes: usize, suffix: &str) -> String {
    if source.len() <= max_bytes {
        return source.to_string();
    }
    let suffix_bytes = suffix.len();
    if suffix_bytes >= max_bytes {
        let mut end = max_bytes.min(source.len());
        while end > 0 && !source.is_char_boundary(end) {
            end -= 1;
        }
        return source[..end].to_string();
    }
    let mut end = (max_bytes - suffix_bytes).min(source.len());
    while end > 0 && !source.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{}", &source[..end], suffix)
}
