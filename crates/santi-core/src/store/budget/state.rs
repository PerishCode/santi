use rusqlite::{OptionalExtension, params};

use crate::store::db::Database;
use santi_provider::ProviderItem;

use crate::{MessageContent, SantiError};

use super::{Pressure, context_incident_key};

impl Database<'_> {
    pub(super) fn pending_items(&self, strand_id: &str) -> Result<Vec<ProviderItem>, String> {
        let mut stmt = self
            .conn
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
            if let Some(item) = crate::context::budget::inbound_provider_item(
                &crate::MessageKind::decode(&kind),
                &content,
            ) {
                items.push(item);
            }
        }
        Ok(items)
    }

    pub(super) fn current_strand_seq(&self, strand_id: &str) -> Result<Option<i64>, String> {
        self.conn
            .query_row(
                "SELECT next_seq - 1 FROM strands WHERE id = ?1 LIMIT 1",
                params![strand_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub(super) fn open_context_incident(
        &self,
        strand_id: &str,
        input: Pressure<'_>,
    ) -> Result<SantiError, String> {
        self.open_incident(input.into_draft(strand_id))
    }

    pub(super) fn repeat_context_incident(
        &self,
        strand_id: &str,
        operation: &str,
    ) -> Result<SantiError, String> {
        let incident_key = context_incident_key(strand_id);
        let existing = self
            .active_incident(&incident_key)?
            .ok_or_else(|| "active context-budget incident missing".to_string())?;
        self.open_incident(santi_error::IncidentDraft {
            incident_key,
            descriptor: santi_error::catalog::CONTEXT_BUDGET_EXCEEDED,
            scope: existing.scope,
            source: santi_error::ErrorSource::new("santi-core", operation),
            message: existing.latest_message,
            context: existing.latest_context,
        })
    }
}
