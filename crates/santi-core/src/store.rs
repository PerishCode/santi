use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use rusqlite::{Connection, params};

use crate::{
    ActorType, IngestOutcome, MessageContent, MessageIntake, MessageKind, MessageState, Strand,
    StrandMessage, StrandSelector, StrandTargetType, Turn, prefixed_id, timestamp_now,
};

mod assembly;
mod compact;
mod db;
mod rows;
mod runtime;
mod schema;

use db::*;
use rows::{actor_type_db, collect_rows, map_webhook_row, message_state_db};
use schema::SCHEMA;

const SANTI_SCHEMA_VERSION: u32 = 19;
const DEFAULT_SOUL_ID: &str = "soul_default";
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

impl SantiStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let conn = Connection::open(path).map_err(|error| error.to_string())?;
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        store.migrate()?;
        store.seed_defaults()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        let version = conn
            .query_row("PRAGMA user_version", [], |row| row.get::<_, u32>(0))
            .map_err(|error| error.to_string())?;
        if version != SANTI_SCHEMA_VERSION {
            conn.execute_batch(
                r#"
                DROP TABLE IF EXISTS response_stream_deltas;
                DROP TABLE IF EXISTS response_runs;
                DROP TABLE IF EXISTS message_text_contents;
                DROP TABLE IF EXISTS conversations;
                DROP TABLE IF EXISTS r_strand_entries;
                DROP TABLE IF EXISTS strand_inbox;
                DROP TABLE IF EXISTS compacts;
                DROP TABLE IF EXISTS thinking_spans;
                DROP TABLE IF EXISTS tool_results;
                DROP TABLE IF EXISTS tool_calls;
                DROP TABLE IF EXISTS turns;
                DROP TABLE IF EXISTS strands;
                DROP TABLE IF EXISTS strand_effects;
                DROP TABLE IF EXISTS message_events;
                DROP TABLE IF EXISTS messages;
                DROP TABLE IF EXISTS webhooks;
                DROP TABLE IF EXISTS soul_profiles;
                DROP TABLE IF EXISTS souls;
                -- Historical (pre-strand-rename) table names, so a clean wipe
                -- reaches a `session`-era DB (e.g. the live box before this
                -- migration). Harmless no-ops on a fresh DB.
                DROP TABLE IF EXISTS soul_sessions;
                DROP TABLE IF EXISTS sessions;
                DROP TABLE IF EXISTS session_profiles;
                DROP TABLE IF EXISTS r_session_messages;
                DROP TABLE IF EXISTS session_effects;
                DROP TABLE IF EXISTS accounts;
                "#,
            )
            .map_err(|error| error.to_string())?;
        }
        conn.execute_batch(SCHEMA)
            .map_err(|error| error.to_string())?;
        conn.pragma_update(None, "user_version", SANTI_SCHEMA_VERSION)
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn seed_defaults(&self) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        let now = timestamp_now();
        // The default soul is id-only; its identity (the first soul = Liberte,
        // the secretary) lives entirely in its memory FILE, which survives a DB
        // wipe. Seeding the row is just "this soul exists". Seeding the initial
        // memory of a fresh instance is config-exposed and lands in STEP 6.
        conn.execute(
            r#"
            INSERT OR IGNORE INTO souls (id, created_at, updated_at)
            VALUES (?1, ?2, ?2)
            "#,
            params![DEFAULT_SOUL_ID, now],
        )
        .map_err(|error| error.to_string())?;
        Ok(())
    }

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
            turns: turns_for_strand(&conn, &strand.id)?,
            thinking_spans: soul_thinking_spans(&conn, &strand.id)?,
            tool_calls: soul_tool_calls(&conn, &strand.id)?,
            tool_results: soul_tool_results(&conn, &strand.id)?,
            compacts: compacts_for_strand(&conn, &strand.id)?,
            effects: strand_effects(&conn, strand_id)?,
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
    pub fn find_or_create_strand_by_label(
        &self,
        soul_id: &str,
        label: &str,
    ) -> Result<Strand, String> {
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
            StrandSelector::ByLabel { soul_id, label } => {
                self.find_or_create_strand_by_label(soul_id, label)
            }
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
        let conn = self.conn.lock().unwrap();
        let pending: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM strand_inbox WHERE strand_id = ?1",
                params![strand_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if pending >= STRAND_INBOX_GATE {
            return Ok(IngestOutcome::Rejected {
                reason: format!(
                    "strand inbox is full ({pending} pending, gate {STRAND_INBOX_GATE})"
                ),
            });
        }
        let inbox_id = prefixed_id("inbox");
        let now = timestamp_now();
        let content_json = serde_json::to_string(&content).map_err(|error| error.to_string())?;
        conn.execute(
            r#"
            INSERT INTO strand_inbox (id, strand_id, message_kind, content, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
            params![
                inbox_id,
                strand_id,
                rows::message_kind_db(&message_kind),
                content_json,
                now
            ],
        )
        .map_err(|error| error.to_string())?;
        Ok(IngestOutcome::Accepted {
            strand_id: strand_id.to_string(),
        })
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
