mod budget;
mod downstream;
pub use downstream::ReplayInsert;
mod event;
pub use event::{Prepared, Transition};
mod inbox;
mod lifecycle;
mod query;
mod turn;
pub use turn::TurnOutboxInsert;

use rusqlite::{Connection, OptionalExtension, params};

use santi_model::{now, soul::Soul, strand::Strand};

use super::rows::*;
pub use event::receipt_state_from_db;
pub use inbox::drain_inbox_in_tx;
pub use lifecycle::{migrate, read_schema_version};
use santi_model::{message, strand, webhook};

pub struct Database<'a> {
    pub(super) conn: &'a Connection,
}

impl<'a> Database<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    pub fn append_entry_in_tx(
        &self,
        strand: &str,
        kind: strand::Target,
        target: &str,
    ) -> Result<strand::Entry, String> {
        let now = now();
        let allocated_seq = self
            .conn
            .query_row(
                r#"
            UPDATE strands
            SET next_seq = next_seq + 1, updated_at = ?2
            WHERE id = ?1
            RETURNING next_seq - 1
            "#,
                params![strand, now],
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
                params![strand, kind.encode(), target, allocated_seq, now],
            )
            .map_err(|error| error.to_string())?;
        Ok(strand::Entry {
            strand: strand.to_string(),
            kind,
            target: target.to_string(),
            seq: allocated_seq,
            created: now,
        })
    }

    pub fn message_events_for_strand(&self, strand: &str) -> Result<Vec<message::Event>, String> {
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
            .query_map(params![strand], message::Event::decode)
            .map_err(|error| error.to_string())?;
        collect_rows(rows)
    }

    pub fn soul_by_id(&self, soul: &str) -> Result<Option<Soul>, String> {
        self.conn
            .query_row(
                r#"
        SELECT id, created_at, updated_at
        FROM souls
        WHERE id = ?1
        LIMIT 1
        "#,
                params![soul],
                Soul::decode,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub fn webhook_by_name(&self, name: &str) -> Result<Option<webhook::Subscription>, String> {
        self.conn
            .query_row(
                r#"
        SELECT name, adaptor, soul_id, strand_strategy, secret_env, created_at, updated_at
        FROM webhooks
        WHERE name = ?1
        LIMIT 1
        "#,
                params![name],
                webhook::Subscription::decode,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub fn strand_by_id(&self, strand: &str) -> Result<Option<Strand>, String> {
        self.conn
            .query_row(
                r#"
        SELECT id, soul_id, external_label, strand_memory, provider_state, next_seq,
               last_seen_strand_seq, parent_strand_id, fork_point, created_at, updated_at
        FROM strands
        WHERE id = ?1
        LIMIT 1
        "#,
                params![strand],
                Strand::decode,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub fn strand_by_label(&self, soul: &str, label: &str) -> Result<Option<Strand>, String> {
        self.conn
            .query_row(
                r#"
        SELECT id, soul_id, external_label, strand_memory, provider_state, next_seq,
               last_seen_strand_seq, parent_strand_id, fork_point, created_at, updated_at
        FROM strands
        WHERE soul_id = ?1 AND external_label = ?2
        LIMIT 1
        "#,
                params![soul, label],
                Strand::decode,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub fn message_by_id(&self, message: &str) -> Result<Option<message::Placed>, String> {
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
                params![message],
                message::Placed::decode,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub fn message_record_by_id(
        &self,
        message: &str,
    ) -> Result<Option<santi_model::message::Message>, String> {
        self.conn
            .query_row(
                r#"
        SELECT id, actor_type, actor_id, message_kind, content, state, version,
               deleted_at, created_at, updated_at
        FROM messages
        WHERE id = ?1
        LIMIT 1
        "#,
                params![message],
                santi_model::message::Message::decode,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub fn strand_messages(&self, strand: &str) -> Result<Vec<message::Placed>, String> {
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
            .query_map(params![strand], message::Placed::decode)
            .map_err(|error| error.to_string())?;
        collect_rows(rows)
    }
}

pub fn message_to_provider_item(
    message: &santi_model::message::Message,
) -> Option<santi_provider::ProviderItem> {
    let role = match (&message.role, &message.kind) {
        (message::Role::Soul, _) => "assistant",
        (message::Role::System, message::Kind::Text) => "user",
        (message::Role::System, message::Kind::SantiSystem) => "system",
    };
    let content = message.content.rendered();
    if content.trim().is_empty() {
        None
    } else {
        Some(santi_provider::ProviderItem::Message {
            role: role.to_string(),
            content,
        })
    }
}
