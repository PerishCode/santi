use std::sync::{Arc, Mutex};

use rusqlite::{Connection, params};

use crate::{
    ActorType, InboxSource, IngestOutcome, MessageContent, MessageIntake, MessageKind,
    MessageState, SantiError, Strand, StrandMessage, StrandSelector, StrandTargetType, Turn,
    prefixed_id, timestamp_now,
};

mod assembly;
pub(crate) mod budget;
mod compact;
mod db;
mod errors;
mod fork;
mod im;
mod rows;
mod runtime;
mod schema;
mod turns;

use db::*;
pub use db::{read_schema_version, soul_memory_file};
pub(crate) use errors::drive::DriveFailureInput;
use rows::{actor_type_db, collect_rows, map_webhook_row, message_state_db};

/// The schema version this binary expects. On open, recognized runtime-schema
/// migrations run in place; an unrecognized mismatch still falls back to the
/// beta wipe + rebuild policy (see PHASE-07 crux #5 / PHASE-09 tier work).
/// Public so ops paths (`santi doctor`) can compare a DB's `user_version` to it
/// WITHOUT opening the store (which would migrate/wipe).
pub const SCHEMA_VERSION: u32 = 25;
/// The default soul's id. Public so offline ops (doctor/seed) can address it
/// without a running service.
pub const DEFAULT_SOUL_ID: &str = "soul_default";
/// The runtime's one system actor identity. No account/user: every non-soul
/// actor speaks as `system`, whether it's a runtime-authored notice (kind
/// santi_system) or opaque world-inbound content (kind text) — the sender's
/// real identity, if any, lives in the content itself, not in this id.
const SANTI_SYSTEM_ACTOR_ID: &str = "santi";
/// Scale safety valve, not a business rule: past this many undrained entries
/// for one strand, ingest starts rejecting instead of growing the queue
/// without bound. The system enforces the gate; handling a rejection (surface
/// it, or silently drop + log) is the adaptor's policy.
const STRAND_INBOX_GATE: i64 = 500;

#[derive(Clone)]
pub struct SantiStore {
    conn: Arc<Mutex<Connection>>,
}

#[derive(Debug, Clone)]
pub struct AppendedMessage {
    pub strand_message: StrandMessage,
}

#[derive(Debug, Clone)]
pub struct StartedTurn {
    pub turn: Turn,
    /// Inbox entries this call committed into the timeline to reach this turn
    /// (empty for the manual/test-only `start_turn`, which does not drain).
    pub drained_messages: Vec<StrandMessage>,
}

pub(crate) enum StartTurnOutcome {
    Started(StartedTurn),
    Running(Turn),
    Idle,
    Held(SantiError),
}

pub(crate) struct ProviderFailureContext<'a> {
    pub provider: &'a str,
    pub model: &'a str,
    pub stage: &'a str,
    pub operation: &'a str,
    pub round: usize,
    pub detail: &'a str,
}

pub(crate) struct RuntimeFailureContext<'a> {
    pub operation: &'a str,
    pub detail: &'a str,
}

impl SantiStore {
    pub fn default_soul_id(&self) -> &'static str {
        DEFAULT_SOUL_ID
    }

    /// The one system actor identity (see `SANTI_SYSTEM_ACTOR_ID`).
    pub fn system_actor_id(&self) -> &'static str {
        SANTI_SYSTEM_ACTOR_ID
    }

    pub fn list_strands(&self) -> Result<Vec<Strand>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                r#"
                SELECT id, soul_id, external_label, strand_memory, provider_state, next_seq,
                       last_seen_strand_seq, parent_strand_id, fork_point, created_at, updated_at
                FROM strands
                ORDER BY updated_at DESC, id DESC
                "#,
            )
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map([], rows::map_strand_row)
            .map_err(|error| error.to_string())?;
        collect_rows(rows)
    }

    /// Create a new strand owned by the runtime's default soul (the
    /// pre-multi-soul-per-strand path CLI `strand create` still uses).
    pub fn create_strand(&self) -> Result<Strand, String> {
        let conn = self.conn.lock().unwrap();
        let strand_id = prefixed_id("ss");
        let now = timestamp_now();
        conn.execute(
            r#"
            INSERT INTO strands (
              id, soul_id, external_label, strand_memory, provider_state, next_seq,
              last_seen_strand_seq, parent_strand_id, fork_point, created_at, updated_at
            )
            VALUES (?1, ?2, NULL, '', NULL, 1, 0, NULL, NULL, ?3, ?3)
            "#,
            params![strand_id, DEFAULT_SOUL_ID, now],
        )
        .map_err(|error| error.to_string())?;
        strand_by_id(&conn, &strand_id)?.ok_or_else(|| "created strand missing".to_string())
    }

    pub fn strand_messages(&self, strand_id: &str) -> Result<Vec<StrandMessage>, String> {
        let conn = self.conn.lock().unwrap();
        strand_messages(&conn, strand_id)
    }

    pub fn runtime_snapshot(
        &self,
        strand_id: &str,
    ) -> Result<Option<crate::StrandRuntimeSnapshot>, String> {
        let conn = self.conn.lock().unwrap();
        let Some(strand) = strand_by_id(&conn, strand_id)? else {
            return Ok(None);
        };
        Ok(Some(crate::StrandRuntimeSnapshot {
            messages: strand_messages(&conn, strand_id)?,
            message_events: message_events_for_strand(&conn, strand_id)?,
            turns: turns_for_strand(&conn, &strand.id)?,
            thinking_spans: soul_thinking_spans(&conn, &strand.id)?,
            tool_calls: soul_tool_calls(&conn, &strand.id)?,
            tool_results: soul_tool_results(&conn, &strand.id)?,
            compacts: compacts_for_strand(&conn, &strand.id)?,
            effects: strand_effects(&conn, strand_id)?,
            errors: errors::list_in_conn(&conn, "strand", strand_id, 100)?,
            strand,
        }))
    }

    pub fn append_message(
        &self,
        strand_id: &str,
        actor_type: ActorType,
        actor_id: &str,
        content: MessageContent,
        state: MessageState,
        intake: MessageIntake,
    ) -> Result<AppendedMessage, String> {
        self.append_message_with_kind(
            strand_id,
            actor_type,
            actor_id,
            MessageKind::Text,
            content,
            state,
            intake,
        )
    }

    pub fn append_santi_system_message(
        &self,
        strand_id: &str,
        content: MessageContent,
        intake: MessageIntake,
    ) -> Result<AppendedMessage, String> {
        self.append_message_with_kind(
            strand_id,
            ActorType::System,
            SANTI_SYSTEM_ACTOR_ID,
            MessageKind::SantiSystem,
            content,
            MessageState::Fixed,
            intake,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn append_message_with_kind(
        &self,
        strand_id: &str,
        actor_type: ActorType,
        actor_id: &str,
        message_kind: MessageKind,
        content: MessageContent,
        state: MessageState,
        intake: MessageIntake,
    ) -> Result<AppendedMessage, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let message_id = prefixed_id("msg");
        let now = timestamp_now();
        let content_json = serde_json::to_string(&content).map_err(|error| error.to_string())?;
        tx.execute(
            r#"
            INSERT INTO messages (
              id, actor_type, actor_id, message_kind, content, state, version, is_request,
              deleted_at, created_at, updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7, NULL, ?8, ?8)
            "#,
            params![
                message_id,
                actor_type_db(&actor_type),
                actor_id,
                rows::message_kind_db(&message_kind),
                content_json,
                message_state_db(&state),
                intake.is_request() as i64,
                now
            ],
        )
        .map_err(|error| error.to_string())?;
        append_entry_in_tx(&tx, strand_id, StrandTargetType::Message, &message_id)?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(AppendedMessage {
            strand_message: message_by_id(&conn, &message_id)?
                .ok_or_else(|| "created message missing".to_string())?,
        })
    }

    /// Find the strand anchored to an opaque external label (scoped to `soul_id`),
    /// or create one and bind it. The label is opaque to core (its meaning lives
    /// in the adaptor); uniqueness is per-soul, enforced by the partial index.
    pub fn find_labeled_strand(&self, soul_id: &str, label: &str) -> Result<Strand, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        if soul_by_id(&tx, soul_id)?.is_none() {
            return Err("soul not found".to_string());
        }
        let strand_id = if let Some(existing) = strand_by_label(&tx, soul_id, label)? {
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
        strand_by_id(&conn, &strand_id)?.ok_or_else(|| "labeled strand missing".to_string())
    }

    /// Resolve an ingest adaptor's `StrandSelector` to a strand, atomically.
    /// The selector IS the addressing strategy (by id — the operator's; by
    /// label find-or-create scoped to a soul — a webhook's); core just runs it.
    pub fn resolve_strand_selector(&self, selector: &StrandSelector) -> Result<Strand, String> {
        match selector {
            StrandSelector::ById(strand_id) => self
                .strand(strand_id)?
                .ok_or_else(|| "strand not found".to_string()),
            StrandSelector::ByLabel { soul_id, label } => self.find_labeled_strand(soul_id, label),
        }
    }

    /// Enqueue inbound content into a strand's durable inbox — the ONE inbound
    /// path (ingest). Does not touch the timeline; the driver drains the inbox
    /// at the next turn boundary (see `try_start_turn`). `Accepted` confirms
    /// durable enqueue only. Past `STRAND_INBOX_GATE` undrained entries, this
    /// is a scale safety valve: reject rather than grow without bound.
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
        self.enqueue_inbox_with_context(strand_id, message_kind, content, source, None)
    }

    /// Register a webhook subscription (API-managed). The secret itself is never
    /// stored — `secret_env` names the env var the adaptor reads at verify time.
    pub fn create_webhook(
        &self,
        name: &str,
        adaptor: &str,
        soul_id: &str,
        strand_strategy: &str,
        secret_env: &str,
    ) -> Result<crate::WebhookSubscription, String> {
        let conn = self.conn.lock().unwrap();
        let now = timestamp_now();
        conn.execute(
            r#"
            INSERT INTO webhooks (
              name, adaptor, soul_id, strand_strategy, secret_env, created_at, updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
            "#,
            params![name, adaptor, soul_id, strand_strategy, secret_env, now],
        )
        .map_err(|error| error.to_string())?;
        webhook_by_name(&conn, name)?.ok_or_else(|| "created webhook missing".to_string())
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
            .query_map([], map_webhook_row)
            .map_err(|error| error.to_string())?;
        collect_rows(rows)
    }

    pub fn webhook(&self, name: &str) -> Result<Option<crate::WebhookSubscription>, String> {
        let conn = self.conn.lock().unwrap();
        webhook_by_name(&conn, name)
    }

    /// Create a new soul (an individual), id-only. Souls are API-managed, never
    /// config. Seeding the soul's initial `[santi-soul]` memory is the caller's
    /// job (the service, which owns the memory FILE) — the store just mints the
    /// identity row.
    pub fn create_soul(&self) -> Result<crate::Soul, String> {
        let conn = self.conn.lock().unwrap();
        let soul_id = prefixed_id("soul");
        let now = timestamp_now();
        conn.execute(
            "INSERT INTO souls (id, created_at, updated_at) VALUES (?1, ?2, ?2)",
            params![soul_id, now],
        )
        .map_err(|error| error.to_string())?;
        soul_by_id(&conn, &soul_id)?.ok_or_else(|| "created soul missing".to_string())
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
            .query_map([], rows::map_soul_row)
            .map_err(|error| error.to_string())?;
        collect_rows(rows)
    }

    pub fn soul(&self, soul_id: &str) -> Result<Option<crate::Soul>, String> {
        let conn = self.conn.lock().unwrap();
        soul_by_id(&conn, soul_id)
    }

    pub fn strand(&self, strand_id: &str) -> Result<Option<Strand>, String> {
        let conn = self.conn.lock().unwrap();
        strand_by_id(&conn, strand_id)
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
            turn: turn_by_id(&conn, &turn_id)?.ok_or_else(|| "created turn missing".to_string())?,
            drained_messages: Vec::new(),
        })
    }
}
