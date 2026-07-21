use crate::{
    CreateWebhookRequest, InboxSource, IngestOutcome, MessageContent, MessageKind, Strand,
    StrandSelector, prefixed_id, timestamp_now,
};
pub(crate) use budget::Ingress;
use rows::{Decode, collect_rows};
use rusqlite::params;

use super::*;

impl SantiStore {
    pub fn find_labeled_strand(&self, soul_id: &str, label: &str) -> Result<Strand, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let database = Database::new(&tx);
        if database.soul_by_id(soul_id)?.is_none() {
            return Err("soul not found".to_string());
        }
        let strand_id = if let Some(existing) = database.strand_by_label(soul_id, label)? {
            existing.id
        } else {
            let strand_id = prefixed_id("ss");
            let now = timestamp_now();
            tx.execute(
                r#"
                INSERT INTO strands (
                  id, soul_id, external_label, strand_memory, provider_state, next_seq,
                  last_seen_strand_seq, parent_strand_id, fork_point, created_at, updated_at
                )
                VALUES (?1, ?2, ?3, '', NULL, 1, 0, NULL, NULL, ?4, ?4)
                "#,
                params![strand_id, soul_id, label, now],
            )
            .map_err(|error| error.to_string())?;
            strand_id
        };
        tx.commit().map_err(|error| error.to_string())?;
        Database::new(&conn)
            .strand_by_id(&strand_id)?
            .ok_or_else(|| "labeled strand missing".to_string())
    }

    pub fn resolve_strand_selector(&self, selector: &StrandSelector) -> Result<Strand, String> {
        match selector {
            StrandSelector::ById(strand_id) => self
                .strand(strand_id)?
                .ok_or_else(|| "strand not found".to_string()),
            StrandSelector::ByLabel { soul_id, label } => self.find_labeled_strand(soul_id, label),
        }
    }

    pub fn enqueue_inbox(
        &self,
        strand_id: &str,
        message_kind: MessageKind,
        content: MessageContent,
    ) -> Result<IngestOutcome, String> {
        self.enqueue_inbox_with_source(strand_id, message_kind, content, None)
    }

    pub fn enqueue_inbox_with_source(
        &self,
        strand_id: &str,
        message_kind: MessageKind,
        content: MessageContent,
        source: Option<InboxSource>,
    ) -> Result<IngestOutcome, String> {
        self.enqueue_inbox_with_context(Ingress {
            strand: strand_id,
            kind: message_kind,
            content,
            source,
            admission: None,
        })
    }

    pub fn create_webhook(
        &self,
        request: CreateWebhookRequest,
    ) -> Result<crate::WebhookSubscription, String> {
        let conn = self.conn.lock().unwrap();
        let now = timestamp_now();
        let strategy = request.strand_strategy.as_deref().unwrap_or("per_thread");
        conn.execute(
            r#"
            INSERT INTO webhooks (
              name, adaptor, soul_id, strand_strategy, secret_env, created_at, updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
            "#,
            params![
                request.name,
                request.adaptor,
                request.soul_id,
                strategy,
                request.secret_env,
                now
            ],
        )
        .map_err(|error| error.to_string())?;
        Database::new(&conn)
            .webhook_by_name(&request.name)?
            .ok_or_else(|| "created webhook missing".to_string())
    }

    pub fn list_webhooks(&self) -> Result<Vec<crate::WebhookSubscription>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                r#"
                SELECT name, adaptor, soul_id, strand_strategy, secret_env, created_at, updated_at
                FROM webhooks ORDER BY created_at ASC, name ASC
                "#,
            )
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map([], crate::WebhookSubscription::decode)
            .map_err(|error| error.to_string())?;
        collect_rows(rows)
    }

    pub fn webhook(&self, name: &str) -> Result<Option<crate::WebhookSubscription>, String> {
        let conn = self.conn.lock().unwrap();
        Database::new(&conn).webhook_by_name(name)
    }

    pub fn create_soul(&self) -> Result<crate::Soul, String> {
        let conn = self.conn.lock().unwrap();
        let soul_id = prefixed_id("soul");
        let now = timestamp_now();
        conn.execute(
            "INSERT INTO souls (id, created_at, updated_at) VALUES (?1, ?2, ?2)",
            params![soul_id, now],
        )
        .map_err(|error| error.to_string())?;
        Database::new(&conn)
            .soul_by_id(&soul_id)?
            .ok_or_else(|| "created soul missing".to_string())
    }

    pub fn list_souls(&self) -> Result<Vec<crate::Soul>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                r#"
                SELECT id, created_at, updated_at
                FROM souls ORDER BY created_at ASC, id ASC
                "#,
            )
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map([], crate::Soul::decode)
            .map_err(|error| error.to_string())?;
        collect_rows(rows)
    }

    pub fn soul(&self, soul_id: &str) -> Result<Option<crate::Soul>, String> {
        let conn = self.conn.lock().unwrap();
        Database::new(&conn).soul_by_id(soul_id)
    }

    pub fn strand(&self, strand_id: &str) -> Result<Option<Strand>, String> {
        let conn = self.conn.lock().unwrap();
        Database::new(&conn).strand_by_id(strand_id)
    }

    pub fn soul_id_for_strand(&self, strand_id: &str) -> Result<String, String> {
        self.strand(strand_id)?
            .map(|strand| strand.soul_id)
            .ok_or_else(|| "strand not found".to_string())
    }

    pub fn start_turn(&self, strand_id: &str, trigger_ref: &str) -> Result<StartedTurn, String> {
        let conn = self.conn.lock().unwrap();
        let turn_id = prefixed_id("turn");
        let now = timestamp_now();
        conn.execute(
            r#"
            INSERT INTO turns (
              id, strand_id, trigger_type, trigger_ref,
              base_strand_seq, end_strand_seq, status, error_text,
              created_at, updated_at, finished_at
            )
            SELECT ?1, id, 'strand_send', ?3, next_seq - 1, NULL, 'running',
                   NULL, ?4, ?4, NULL
            FROM strands
            WHERE id = ?2
            "#,
            params![turn_id, strand_id, trigger_ref, now],
        )
        .map_err(|error| error.to_string())?;
        Ok(StartedTurn {
            turn: Database::new(&conn)
                .turn_by_id(&turn_id)?
                .ok_or_else(|| "created turn missing".to_string())?,
            drained_messages: Vec::new(),
        })
    }
}
