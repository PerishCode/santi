mod timeline;

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

use crate::{
    ActorType, Compact, MessageEvent, MessageKind, Soul, Strand, StrandEffect, StrandEntry,
    StrandMessage, StrandTargetType, ThinkingSpan, ToolCall, ToolResult, Turn, WebhookSubscription,
    prefixed_id, timestamp_now,
};

use super::rows::*;
pub(super) use timeline::*;

pub(super) fn append_entry_in_tx(
    conn: &Connection,
    strand_id: &str,
    target_type: StrandTargetType,
    target_id: &str,
) -> Result<StrandEntry, String> {
    let now = timestamp_now();
    let allocated_seq = conn
        .query_row(
            r#"
            UPDATE strands
            SET next_seq = next_seq + 1, updated_at = ?2
            WHERE id = ?1
            RETURNING next_seq - 1
            "#,
            params![strand_id, now],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| error.to_string())?;
    conn.execute(
        r#"
        INSERT INTO r_strand_entries (
          strand_id, target_type, target_id, strand_seq, created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
        params![
            strand_id,
            entry_type_db(&target_type),
            target_id,
            allocated_seq,
            now
        ],
    )
    .map_err(|error| error.to_string())?;
    Ok(StrandEntry {
        strand_id: strand_id.to_string(),
        target_type,
        target_id: target_id.to_string(),
        strand_seq: allocated_seq,
        created_at: now,
    })
}

/// Drain a strand's entire inbox into its timeline: each entry becomes a
/// `messages` row (actor System, `is_request=1`, state fixed) referenced into
/// `r_strand_entries` in arrival order, then the inbox row is removed. This is
/// the ONE place inbound content is committed — ingest itself only durably
/// enqueues. Returns the drained messages (empty ⟺ nothing was pending).
pub(super) fn drain_inbox_in_tx(
    conn: &Connection,
    strand_id: &str,
    committing_turn_id: &str,
) -> Result<Vec<StrandMessage>, String> {
    let mut stmt = conn
        .prepare(
            r#"
            SELECT id, message_kind, content, source_type, source_ref, source_metadata, created_at
            FROM strand_inbox
            WHERE strand_id = ?1
            ORDER BY rowid ASC
            "#,
        )
        .map_err(|error| error.to_string())?;
    let pending = stmt
        .query_map(params![strand_id], |row| {
            Ok(PendingInboxEntry {
                id: row.get(0)?,
                message_kind: row.get(1)?,
                content: row.get(2)?,
                source_type: row.get(3)?,
                source_ref: row.get(4)?,
                source_metadata: row.get(5)?,
                enqueued_at: row.get(6)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    drop(stmt);

    let now = timestamp_now();
    let mut drained = Vec::with_capacity(pending.len());
    for pending_entry in pending {
        let message_id = prefixed_id("msg");
        conn.execute(
            r#"
            INSERT INTO messages (
              id, actor_type, actor_id, message_kind, content, state, version, is_request,
              deleted_at, created_at, updated_at
            )
            VALUES (?1, 'system', ?2, ?3, ?4, 'fixed', 1, 1, NULL, ?5, ?5)
            "#,
            params![
                message_id,
                super::SANTI_SYSTEM_ACTOR_ID,
                pending_entry.message_kind.as_str(),
                pending_entry.content.as_str(),
                now
            ],
        )
        .map_err(|error| error.to_string())?;
        let relation = append_entry_in_tx(conn, strand_id, StrandTargetType::Message, &message_id)?;
        insert_inbox_drain_event_in_tx(
            conn,
            &pending_entry,
            &message_id,
            relation.strand_seq,
            committing_turn_id,
            &now,
        )?;
        conn.execute(
            "DELETE FROM strand_inbox WHERE id = ?1",
            params![pending_entry.id],
        )
        .map_err(|error| error.to_string())?;
        drained.push(
            message_by_id(conn, &message_id)?
                .ok_or_else(|| "drained message missing".to_string())?,
        );
    }
    Ok(drained)
}

struct PendingInboxEntry {
    id: String,
    message_kind: String,
    content: String,
    source_type: Option<String>,
    source_ref: Option<String>,
    source_metadata: Option<String>,
    enqueued_at: String,
}

fn insert_inbox_drain_event_in_tx(
    conn: &Connection,
    pending: &PendingInboxEntry,
    message_id: &str,
    strand_seq: i64,
    turn_id: &str,
    drained_at: &str,
) -> Result<(), String> {
    let source_metadata = pending.source_metadata.as_deref().map(|raw| {
        serde_json::from_str::<Value>(raw).unwrap_or_else(|_| json!({ "invalid_json": true }))
    });
    let payload = json!({
        "kind": "inbox_drain",
        "inbox_id": pending.id.as_str(),
        "enqueued_at": pending.enqueued_at.as_str(),
        "drained_at": drained_at,
        "committing_turn_id": turn_id,
        "message_id": message_id,
        "strand_seq": strand_seq,
        "source": {
            "type": pending.source_type.as_deref(),
            "ref": pending.source_ref.as_deref(),
            "metadata": source_metadata,
        }
    });
    conn.execute(
        r#"
        INSERT INTO message_events (
          id, message_id, action, actor_type, actor_id, base_version, payload, created_at
        )
        VALUES (?1, ?2, 'insert', 'system', ?3, 1, ?4, ?5)
        "#,
        params![
            prefixed_id("mev"),
            message_id,
            super::SANTI_SYSTEM_ACTOR_ID,
            payload.to_string(),
            drained_at
        ],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) fn message_events_for_strand(
    conn: &Connection,
    strand_id: &str,
) -> Result<Vec<MessageEvent>, String> {
    let mut stmt = conn
        .prepare(
            r#"
            SELECT e.id, e.message_id, e.action, e.actor_type, e.actor_id,
                   e.base_version, e.payload, e.created_at
            FROM message_events e
            JOIN r_strand_entries r ON r.target_type = 'message' AND r.target_id = e.message_id
            WHERE r.strand_id = ?1
            ORDER BY r.strand_seq ASC, e.created_at ASC, e.id ASC
            "#,
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(params![strand_id], map_message_event_row)
        .map_err(|error| error.to_string())?;
    collect_rows(rows)
}

pub(super) fn soul_by_id(conn: &Connection, soul_id: &str) -> Result<Option<Soul>, String> {
    conn.query_row(
        r#"
        SELECT id, created_at, updated_at
        FROM souls
        WHERE id = ?1
        LIMIT 1
        "#,
        params![soul_id],
        map_soul_row,
    )
    .optional()
    .map_err(|error| error.to_string())
}

pub(super) fn webhook_by_name(
    conn: &Connection,
    name: &str,
) -> Result<Option<WebhookSubscription>, String> {
    conn.query_row(
        r#"
        SELECT name, adaptor, soul_id, strand_strategy, secret_env, created_at, updated_at
        FROM webhooks
        WHERE name = ?1
        LIMIT 1
        "#,
        params![name],
        map_webhook_row,
    )
    .optional()
    .map_err(|error| error.to_string())
}

pub(super) fn strand_by_id(conn: &Connection, strand_id: &str) -> Result<Option<Strand>, String> {
    conn.query_row(
        r#"
        SELECT id, soul_id, external_label, strand_memory, provider_state, next_seq,
               last_seen_strand_seq, parent_strand_id, fork_point, created_at, updated_at
        FROM strands
        WHERE id = ?1
        LIMIT 1
        "#,
        params![strand_id],
        map_strand_row,
    )
    .optional()
    .map_err(|error| error.to_string())
}

pub(super) fn strand_by_label(
    conn: &Connection,
    soul_id: &str,
    label: &str,
) -> Result<Option<Strand>, String> {
    conn.query_row(
        r#"
        SELECT id, soul_id, external_label, strand_memory, provider_state, next_seq,
               last_seen_strand_seq, parent_strand_id, fork_point, created_at, updated_at
        FROM strands
        WHERE soul_id = ?1 AND external_label = ?2
        LIMIT 1
        "#,
        params![soul_id, label],
        map_strand_row,
    )
    .optional()
    .map_err(|error| error.to_string())
}

pub(super) fn message_by_id(
    conn: &Connection,
    message_id: &str,
) -> Result<Option<StrandMessage>, String> {
    conn.query_row(
        r#"
        SELECT r.strand_id, r.target_id, r.strand_seq, r.created_at,
               m.id, m.actor_type, m.actor_id, m.message_kind, m.content, m.state, m.version,
               m.deleted_at, m.created_at, m.updated_at
        FROM r_strand_entries r
        JOIN messages m ON m.id = r.target_id
        WHERE r.target_type = 'message' AND r.target_id = ?1
        LIMIT 1
        "#,
        params![message_id],
        map_strand_message_row,
    )
    .optional()
    .map_err(|error| error.to_string())
}

/// Fetch a message's content by id directly from `messages`, independent of any
/// strand relation — so the assembly projection can render both timeline-visible
/// messages and strand-only assistant text items uniformly.
pub(super) fn message_record_by_id(
    conn: &Connection,
    message_id: &str,
) -> Result<Option<crate::Message>, String> {
    conn.query_row(
        r#"
        SELECT id, actor_type, actor_id, message_kind, content, state, version,
               deleted_at, created_at, updated_at
        FROM messages
        WHERE id = ?1
        LIMIT 1
        "#,
        params![message_id],
        map_message_row,
    )
    .optional()
    .map_err(|error| error.to_string())
}

pub(super) fn strand_messages(
    conn: &Connection,
    strand_id: &str,
) -> Result<Vec<StrandMessage>, String> {
    let mut stmt = conn
        .prepare(
            r#"
            SELECT r.strand_id, r.target_id, r.strand_seq, r.created_at,
                   m.id, m.actor_type, m.actor_id, m.message_kind, m.content, m.state, m.version,
                   m.deleted_at, m.created_at, m.updated_at
            FROM r_strand_entries r
            JOIN messages m ON m.id = r.target_id
            WHERE r.strand_id = ?1 AND r.target_type = 'message' AND m.deleted_at IS NULL
            ORDER BY r.strand_seq ASC
            "#,
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(params![strand_id], map_strand_message_row)
        .map_err(|error| error.to_string())?;
    collect_rows(rows)
}

pub(super) fn turn_by_id(conn: &Connection, turn_id: &str) -> Result<Option<Turn>, String> {
    conn.query_row(
        r#"
        SELECT id, strand_id, trigger_type, trigger_ref,
               base_strand_seq, end_strand_seq, status, error_text,
               created_at, updated_at, finished_at
        FROM turns
        WHERE id = ?1
        LIMIT 1
        "#,
        params![turn_id],
        map_turn_row,
    )
    .optional()
    .map_err(|error| error.to_string())
}

pub(super) fn compact_by_id(
    conn: &Connection,
    compact_id: &str,
) -> Result<Option<Compact>, String> {
    conn.query_row(
        r#"
        SELECT id, strand_id, summary, start_message_id, end_message_id
        FROM compacts WHERE id = ?1 LIMIT 1
        "#,
        params![compact_id],
        map_compact_row,
    )
    .optional()
    .map_err(|error| error.to_string())
}

pub(super) fn turn_strand_id(conn: &Connection, turn_id: &str) -> Result<String, String> {
    conn.query_row(
        "SELECT strand_id FROM turns WHERE id = ?1 LIMIT 1",
        params![turn_id],
        |row| row.get(0),
    )
    .map_err(|error| error.to_string())
}

pub(super) fn call_soul_id(conn: &Connection, tool_call_id: &str) -> Result<String, String> {
    conn.query_row(
        r#"
        SELECT t.strand_id
        FROM tool_calls c
        JOIN turns t ON t.id = c.turn_id
        WHERE c.id = ?1
        LIMIT 1
        "#,
        params![tool_call_id],
        |row| row.get(0),
    )
    .map_err(|error| error.to_string())
}

pub(super) fn tool_call_by_id(
    conn: &Connection,
    tool_call_id: &str,
) -> Result<Option<ToolCall>, String> {
    conn.query_row(
        "SELECT id, turn_id, tool_name, arguments, created_at FROM tool_calls WHERE id = ?1 LIMIT 1",
        params![tool_call_id],
        map_tool_call_row,
    )
    .optional()
    .map_err(|error| error.to_string())
}

/// Load a tool_call's REGENERABLE provider replay material (raw wire item +
/// item_id), if any. The blob is advisory: the caller (an adaptor) still
/// validates it and, if invalid, regenerates from the neutral tool_call fields.
/// Irreplaceable material is deliberately NOT returned here — it must never be
/// treated as regenerable (PHASE-09 decision #9).
pub(super) fn regenerable_replay_material(
    conn: &Connection,
    tool_call_id: &str,
) -> Result<(Option<serde_json::Value>, Option<String>), String> {
    conn.query_row(
        "SELECT blob, item_id FROM provider_replay_material \
         WHERE tool_call_id = ?1 AND kind = 'regenerable' LIMIT 1",
        params![tool_call_id],
        |row| {
            let blob: Option<String> = row.get(0)?;
            let item_id: Option<String> = row.get(1)?;
            Ok((blob, item_id))
        },
    )
    .optional()
    .map_err(|error| error.to_string())
    .map(|found| match found {
        Some((blob, item_id)) => (blob.and_then(|b| serde_json::from_str(&b).ok()), item_id),
        None => (None, None),
    })
}

pub(super) fn tool_result_by_id(
    conn: &Connection,
    tool_result_id: &str,
) -> Result<Option<ToolResult>, String> {
    conn.query_row(
        "SELECT id, tool_call_id, output, error_text, created_at FROM tool_results WHERE id = ?1 LIMIT 1",
        params![tool_result_id],
        map_tool_result_row,
    )
    .optional()
    .map_err(|error| error.to_string())
}

pub(super) fn thinking_span_by_id(
    conn: &Connection,
    thinking_span_id: &str,
) -> Result<Option<ThinkingSpan>, String> {
    conn.query_row(
        r#"
        SELECT id, turn_id, provider_response_id, state, summary, completion_reason,
               error_text, created_at, updated_at, finished_at
        FROM thinking_spans
        WHERE id = ?1
        LIMIT 1
        "#,
        params![thinking_span_id],
        map_thinking_span_row,
    )
    .optional()
    .map_err(|error| error.to_string())
}

/// Position of a message in a strand's spine (its ref's strand_seq),
/// or None if the message is not part of that strand. This is the one
/// axis compaction operates on — message_id in, strand_seq out.
pub(super) fn message_seq_in_strand(
    conn: &Connection,
    strand_id: &str,
    message_id: &str,
) -> Result<Option<i64>, String> {
    conn.query_row(
        r#"
        SELECT strand_seq FROM r_strand_entries
        WHERE strand_id = ?1 AND target_type = 'message' AND target_id = ?2
        LIMIT 1
        "#,
        params![strand_id, message_id],
        |row| row.get::<_, i64>(0),
    )
    .optional()
    .map_err(|error| error.to_string())
}

pub(super) fn compacts_for_strand(
    conn: &Connection,
    strand_id: &str,
) -> Result<Vec<Compact>, String> {
    let mut stmt = conn
        .prepare(
            r#"
            SELECT id, strand_id, summary, start_message_id, end_message_id
            FROM compacts
            WHERE strand_id = ?1
            "#,
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(params![strand_id], map_compact_row)
        .map_err(|error| error.to_string())?;
    collect_rows(rows)
}

pub(super) fn strand_effects(
    conn: &Connection,
    strand_id: &str,
) -> Result<Vec<StrandEffect>, String> {
    let mut stmt = conn
        .prepare(
            r#"
            SELECT id, strand_id, effect_type, idempotency_key, status, source_hook_id,
                   source_turn_id, result_ref, error_text, created_at, updated_at
            FROM strand_effects
            WHERE strand_id = ?1
            ORDER BY created_at ASC
            "#,
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(params![strand_id], map_strand_effect_row)
        .map_err(|error| error.to_string())?;
    collect_rows(rows)
}

/// The provider boundary: `(actor, message_kind)` is the whole marker, no
/// separate column. Soul always speaks as `assistant`. System splits by kind:
/// `Text` is opaque world-inbound content (a CLI send, a webhook event) → the
/// provider hears it as `user`; `SantiSystem` is a runtime-authored fact about
/// this strand (not user speech, see the `<system_message>` prompt copy) → `system`.
pub(super) fn message_to_provider_item(
    message: &crate::Message,
) -> Option<santi_provider::ProviderItem> {
    let role = match (&message.actor_type, &message.message_kind) {
        (ActorType::Soul, _) => "assistant",
        (ActorType::System, MessageKind::Text) => "user",
        (ActorType::System, MessageKind::SantiSystem) => "system",
    };
    let content = message.content.content_text();
    if content.trim().is_empty() {
        None
    } else {
        Some(santi_provider::ProviderItem::Message {
            role: role.to_string(),
            content,
        })
    }
}
