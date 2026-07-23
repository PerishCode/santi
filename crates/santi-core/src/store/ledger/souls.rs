use crate::{now, strand::Strand, tag};
pub(crate) use budget::Ingress;
use rows::{Decode, collect_rows};
use rusqlite::params;

use super::{SantiStore, db::Database};
use crate::store::{StartedTurn, budget, rows};
use crate::{ingest, message, strand, webhook};

impl SantiStore {
    pub fn find_labeled_strand(&self, soul: &str, label: &str) -> Result<Strand, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let database = Database::new(&tx);
        if database.soul_by_id(soul)?.is_none() {
            return Err("soul not found".to_string());
        }
        let strand = if let Some(existing) = database.strand_by_label(soul, label)? {
            existing.id
        } else {
            let strand = tag("ss");
            let now = now();
            tx.execute(
                r#"
                INSERT INTO strands (
                  id, soul_id, external_label, strand_memory, provider_state, next_seq,
                  last_seen_strand_seq, parent_strand_id, fork_point, created_at, updated_at
                )
                VALUES (?1, ?2, ?3, '', NULL, 1, 0, NULL, NULL, ?4, ?4)
                "#,
                params![strand, soul, label, now],
            )
            .map_err(|error| error.to_string())?;
            strand
        };
        tx.commit().map_err(|error| error.to_string())?;
        Database::new(&conn)
            .strand_by_id(&strand)?
            .ok_or_else(|| "labeled strand missing".to_string())
    }

    pub fn resolve_strand_selector(&self, selector: &strand::Selector) -> Result<Strand, String> {
        match selector {
            strand::Selector::ById(strand) => self
                .strand(strand)?
                .ok_or_else(|| "strand not found".to_string()),
            strand::Selector::ByLabel { soul, label } => self.find_labeled_strand(soul, label),
        }
    }

    pub fn enqueue_inbox(
        &self,
        strand: &str,
        kind: message::Kind,
        content: message::Content,
    ) -> Result<ingest::Outcome, String> {
        self.enqueue_inbox_with_source(strand, kind, content, None)
    }

    pub fn enqueue_inbox_with_source(
        &self,
        strand: &str,
        kind: message::Kind,
        content: message::Content,
        source: Option<ingest::Source>,
    ) -> Result<ingest::Outcome, String> {
        self.enqueue_inbox_with_context(Ingress {
            strand,
            kind,
            content,
            source,
            admission: None,
            replay: None,
        })
        .map(|intake| intake.outcome)
    }

    pub fn create_webhook(
        &self,
        request: webhook::Draft,
    ) -> Result<crate::webhook::Subscription, String> {
        let conn = self.conn.lock().unwrap();
        let now = now();
        let strategy = request.strategy.as_deref().unwrap_or("per_thread");
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
                request.soul,
                strategy,
                request.credential,
                now
            ],
        )
        .map_err(|error| error.to_string())?;
        Database::new(&conn)
            .webhook_by_name(&request.name)?
            .ok_or_else(|| "created webhook missing".to_string())
    }

    pub fn list_webhooks(&self) -> Result<Vec<crate::webhook::Subscription>, String> {
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
            .query_map([], crate::webhook::Subscription::decode)
            .map_err(|error| error.to_string())?;
        collect_rows(rows)
    }

    pub fn webhook(&self, name: &str) -> Result<Option<crate::webhook::Subscription>, String> {
        let conn = self.conn.lock().unwrap();
        Database::new(&conn).webhook_by_name(name)
    }

    pub fn create_soul(&self) -> Result<crate::soul::Soul, String> {
        let conn = self.conn.lock().unwrap();
        let soul = tag("soul");
        let now = now();
        conn.execute(
            "INSERT INTO souls (id, created_at, updated_at) VALUES (?1, ?2, ?2)",
            params![soul, now],
        )
        .map_err(|error| error.to_string())?;
        Database::new(&conn)
            .soul_by_id(&soul)?
            .ok_or_else(|| "created soul missing".to_string())
    }

    pub fn list_souls(&self) -> Result<Vec<crate::soul::Soul>, String> {
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
            .query_map([], crate::soul::Soul::decode)
            .map_err(|error| error.to_string())?;
        collect_rows(rows)
    }

    pub fn soul(&self, soul: &str) -> Result<Option<crate::soul::Soul>, String> {
        let conn = self.conn.lock().unwrap();
        Database::new(&conn).soul_by_id(soul)
    }

    pub fn strand(&self, strand: &str) -> Result<Option<Strand>, String> {
        let conn = self.conn.lock().unwrap();
        Database::new(&conn).strand_by_id(strand)
    }

    pub fn soul_id_for_strand(&self, strand: &str) -> Result<String, String> {
        self.strand(strand)?
            .map(|strand| strand.soul)
            .ok_or_else(|| "strand not found".to_string())
    }

    pub fn start_turn(&self, strand: &str, source: &str) -> Result<StartedTurn, String> {
        let conn = self.conn.lock().unwrap();
        let turn = tag("turn");
        let now = now();
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
            params![turn, strand, source, now],
        )
        .map_err(|error| error.to_string())?;
        Ok(StartedTurn {
            turn: Database::new(&conn)
                .turn_by_id(&turn)?
                .ok_or_else(|| "created turn missing".to_string())?,
            drained_messages: Vec::new(),
        })
    }
}
