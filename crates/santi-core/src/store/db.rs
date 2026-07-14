mod inbox;
mod lifecycle;
mod migration;
mod receipts;
mod timeline;

use rusqlite::{Connection, OptionalExtension, params};

use crate::{
    ActorType, Compact, MessageEvent, MessageKind, Soul, Strand, StrandEffect, StrandEntry,
    StrandMessage, StrandTargetType, ThinkingSpan, ToolCall, ToolResult, Turn, WebhookSubscription,
    timestamp_now,
};

use super::rows::*;
pub(super) use inbox::drain_inbox_in_tx;
pub use lifecycle::{read_schema_version, soul_memory_file};

pub(super) struct Database<'a> {
    pub(super) conn: &'a Connection,
}

impl<'a> Database<'a> {
    pub(super) fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    pub(super) fn append_entry_in_tx(
        &self,
        strand_id: &str,
        target_type: StrandTargetType,
        target_id: &str,
    ) -> Result<StrandEntry, String> {
        let now = timestamp_now();
        let allocated_seq = self
            .conn
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
        self.conn
            .execute(
                r#"
        INSERT INTO r_strand_entries (
          strand_id, target_type, target_id, strand_seq, created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
                params![
                    strand_id,
                    target_type.encode(),
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

    pub(super) fn message_events_for_strand(
        &self,
        strand_id: &str,
    ) -> Result<Vec<MessageEvent>, String> {
        let mut stmt = self
            .conn
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
            .query_map(params![strand_id], MessageEvent::decode)
            .map_err(|error| error.to_string())?;
        collect_rows(rows)
    }

    pub(super) fn soul_by_id(&self, soul_id: &str) -> Result<Option<Soul>, String> {
        self.conn
            .query_row(
                r#"
        SELECT id, created_at, updated_at
        FROM souls
        WHERE id = ?1
        LIMIT 1
        "#,
                params![soul_id],
                Soul::decode,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub(super) fn webhook_by_name(
        &self,
        name: &str,
    ) -> Result<Option<WebhookSubscription>, String> {
        self.conn
            .query_row(
                r#"
        SELECT name, adaptor, soul_id, strand_strategy, secret_env, created_at, updated_at
        FROM webhooks
        WHERE name = ?1
        LIMIT 1
        "#,
                params![name],
                WebhookSubscription::decode,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub(super) fn strand_by_id(&self, strand_id: &str) -> Result<Option<Strand>, String> {
        self.conn
            .query_row(
                r#"
        SELECT id, soul_id, external_label, strand_memory, provider_state, next_seq,
               last_seen_strand_seq, parent_strand_id, fork_point, created_at, updated_at
        FROM strands
        WHERE id = ?1
        LIMIT 1
        "#,
                params![strand_id],
                Strand::decode,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub(super) fn strand_by_label(
        &self,
        soul_id: &str,
        label: &str,
    ) -> Result<Option<Strand>, String> {
        self.conn
            .query_row(
                r#"
        SELECT id, soul_id, external_label, strand_memory, provider_state, next_seq,
               last_seen_strand_seq, parent_strand_id, fork_point, created_at, updated_at
        FROM strands
        WHERE soul_id = ?1 AND external_label = ?2
        LIMIT 1
        "#,
                params![soul_id, label],
                Strand::decode,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub(super) fn message_by_id(&self, message_id: &str) -> Result<Option<StrandMessage>, String> {
        self.conn
            .query_row(
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
                StrandMessage::decode,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub(super) fn message_record_by_id(
        &self,
        message_id: &str,
    ) -> Result<Option<crate::Message>, String> {
        self.conn
            .query_row(
                r#"
        SELECT id, actor_type, actor_id, message_kind, content, state, version,
               deleted_at, created_at, updated_at
        FROM messages
        WHERE id = ?1
        LIMIT 1
        "#,
                params![message_id],
                crate::Message::decode,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub(super) fn strand_messages(&self, strand_id: &str) -> Result<Vec<StrandMessage>, String> {
        let mut stmt = self
            .conn
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
            .query_map(params![strand_id], StrandMessage::decode)
            .map_err(|error| error.to_string())?;
        collect_rows(rows)
    }

    pub(super) fn turn_by_id(&self, turn_id: &str) -> Result<Option<Turn>, String> {
        self.conn
            .query_row(
                r#"
        SELECT id, strand_id, trigger_type, trigger_ref,
               base_strand_seq, end_strand_seq, status, error_text,
               created_at, updated_at, finished_at
        FROM turns
        WHERE id = ?1
        LIMIT 1
        "#,
                params![turn_id],
                Turn::decode,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub(super) fn compact_by_id(&self, compact_id: &str) -> Result<Option<Compact>, String> {
        self.conn
            .query_row(
                r#"
        SELECT id, strand_id, summary, start_message_id, end_message_id, created_at, metadata
        FROM compacts WHERE id = ?1 LIMIT 1
        "#,
                params![compact_id],
                Compact::decode,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub(super) fn turn_strand_id(&self, turn_id: &str) -> Result<String, String> {
        self.conn
            .query_row(
                "SELECT strand_id FROM turns WHERE id = ?1 LIMIT 1",
                params![turn_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())
    }

    pub(super) fn call_soul_id(&self, tool_call_id: &str) -> Result<String, String> {
        self.conn
            .query_row(
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

    pub(super) fn tool_call_by_id(&self, tool_call_id: &str) -> Result<Option<ToolCall>, String> {
        self.conn.query_row(
        "SELECT id, turn_id, tool_name, arguments, created_at FROM tool_calls WHERE id = ?1 LIMIT 1",
        params![tool_call_id],
        ToolCall::decode,
    )
    .optional()
    .map_err(|error| error.to_string())
    }

    pub(super) fn regenerable_replay_material(
        &self,
        tool_call_id: &str,
    ) -> Result<(Option<serde_json::Value>, Option<String>), String> {
        self.conn
            .query_row(
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
                Some((blob, item_id)) => {
                    (blob.and_then(|b| serde_json::from_str(&b).ok()), item_id)
                }
                None => (None, None),
            })
    }

    pub(super) fn tool_result_by_id(
        &self,
        tool_result_id: &str,
    ) -> Result<Option<ToolResult>, String> {
        self.conn.query_row(
        "SELECT id, tool_call_id, output, error_text, created_at FROM tool_results WHERE id = ?1 LIMIT 1",
        params![tool_result_id],
        ToolResult::decode,
    )
    .optional()
    .map_err(|error| error.to_string())
    }

    pub(super) fn thinking_span_by_id(
        &self,
        thinking_span_id: &str,
    ) -> Result<Option<ThinkingSpan>, String> {
        self.conn
            .query_row(
                r#"
        SELECT id, turn_id, provider_response_id, state, summary, completion_reason,
               error_text, created_at, updated_at, finished_at
        FROM thinking_spans
        WHERE id = ?1
        LIMIT 1
        "#,
                params![thinking_span_id],
                ThinkingSpan::decode,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub(super) fn message_seq_in_strand(
        &self,
        strand_id: &str,
        message_id: &str,
    ) -> Result<Option<i64>, String> {
        self.conn
            .query_row(
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

    pub(super) fn compacts_for_strand(&self, strand_id: &str) -> Result<Vec<Compact>, String> {
        let mut stmt = self
            .conn
            .prepare(
                r#"
            SELECT id, strand_id, summary, start_message_id, end_message_id, created_at, metadata
            FROM compacts
            WHERE strand_id = ?1
            "#,
            )
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map(params![strand_id], Compact::decode)
            .map_err(|error| error.to_string())?;
        collect_rows(rows)
    }

    pub(super) fn strand_effects(&self, strand_id: &str) -> Result<Vec<StrandEffect>, String> {
        let mut stmt = self
            .conn
            .prepare(
                r#"
            SELECT id, strand_id, turn_id, tool_call_id, effect_type, state,
                   result_ref, error_text, created_at, updated_at, dispatched_at, settled_at
            FROM strand_effects
            WHERE strand_id = ?1
            ORDER BY created_at ASC
            "#,
            )
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map(params![strand_id], StrandEffect::decode)
            .map_err(|error| error.to_string())?;
        collect_rows(rows)
    }
}

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
