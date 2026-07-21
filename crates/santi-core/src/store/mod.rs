use std::sync::{Arc, Mutex};

use rusqlite::{Connection, params};

use crate::{
    ActorType, MessageContent, MessageIntake, MessageKind, MessageState, SantiError, Strand,
    StrandMessage, StrandTargetType, Turn, prefixed_id, timestamp_now,
};

mod assembly;
pub(crate) mod budget;
mod compact;
mod db;
mod effects;
pub(crate) mod errors;
mod fork;
mod im;
mod lifecycle;
mod receipts;
mod reply;
mod rows;
mod runtime;
mod souls;
mod span;
mod turns;

pub(crate) use budget::{Ingress, Launch, execution_budget_incident_key};
pub(crate) use compact::Collapse;
pub use db::read_schema_version;
use db::*;
pub(crate) use effects::Settlement;
pub use im::Reply;
use rows::{Decode, collect_rows};
pub use runtime::Invocation;
pub use turns::Completion;

use santi_adaptor::SANTI_SYSTEM_ACTOR_ID;
pub use santi_adaptor::SCHEMA_VERSION;
pub const DEFAULT_SOUL_ID: &str = "soul_default";
const STRAND_INBOX_GATE: i64 = 500;

pub fn soul_memory_file(
    runtime_root: impl AsRef<std::path::Path>,
    soul_id: &str,
) -> std::path::PathBuf {
    runtime_root
        .as_ref()
        .join("souls")
        .join(soul_id)
        .join("memory")
        .join(crate::workspace::MEMORY_FILE)
}

#[derive(Clone)]
pub struct SantiStore {
    conn: Arc<Mutex<Connection>>,
    im: santi_im::ImStore,
}

#[derive(Debug, Clone)]
pub struct AppendedMessage {
    pub strand_message: StrandMessage,
}

pub struct Draft<'a> {
    pub strand: &'a str,
    pub actor: ActorType,
    pub id: &'a str,
    pub content: MessageContent,
    pub state: MessageState,
    pub intake: MessageIntake,
}

#[derive(Debug, Clone)]
pub struct StartedTurn {
    pub turn: Turn,
    pub drained_messages: Vec<StrandMessage>,
}

pub(crate) enum StartTurnOutcome {
    Started(StartedTurn),
    Running(Turn),
    Idle,
    Held(SantiError),
}

pub(crate) struct ProviderFault<'a> {
    pub provider: &'a str,
    pub model: &'a str,
    pub stage: &'a str,
    pub operation: &'a str,
    pub round: usize,
    pub detail: &'a str,
}

pub(crate) struct RuntimeFault<'a> {
    pub operation: &'a str,
    pub detail: &'a str,
}

impl SantiStore {
    pub fn default_soul_id(&self) -> &'static str {
        DEFAULT_SOUL_ID
    }

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
            .query_map([], Strand::decode)
            .map_err(|error| error.to_string())?;
        collect_rows(rows)
    }

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
        Database::new(&conn)
            .strand_by_id(&strand_id)?
            .ok_or_else(|| "created strand missing".to_string())
    }

    pub fn strand_messages(&self, strand_id: &str) -> Result<Vec<StrandMessage>, String> {
        let conn = self.conn.lock().unwrap();
        Database::new(&conn).strand_messages(strand_id)
    }

    pub fn runtime_snapshot(
        &self,
        strand_id: &str,
    ) -> Result<Option<crate::StrandRuntimeSnapshot>, String> {
        let conn = self.conn.lock().unwrap();
        let database = Database::new(&conn);
        let Some(strand) = database.strand_by_id(strand_id)? else {
            return Ok(None);
        };
        Ok(Some(crate::StrandRuntimeSnapshot {
            messages: database.strand_messages(strand_id)?,
            message_events: database.message_events_for_strand(strand_id)?,
            turns: database.turns_for_strand(&strand.id)?,
            thinking_spans: database.soul_thinking_spans(&strand.id)?,
            tool_calls: database.soul_tool_calls(&strand.id)?,
            tool_results: database.soul_tool_results(&strand.id)?,
            compacts: database.compacts_for_strand(&strand.id)?,
            effects: database.strand_effects(strand_id)?,
            errors: database.list_incidents("strand", strand_id, 100)?,
            strand,
        }))
    }

    pub fn append_message(&self, draft: Draft<'_>) -> Result<AppendedMessage, String> {
        self.append_message_with_kind(draft, MessageKind::Text)
    }

    pub fn append_santi_system_message(
        &self,
        strand_id: &str,
        content: MessageContent,
        intake: MessageIntake,
    ) -> Result<AppendedMessage, String> {
        self.append_message_with_kind(
            Draft {
                strand: strand_id,
                actor: ActorType::System,
                id: SANTI_SYSTEM_ACTOR_ID,
                content,
                state: MessageState::Fixed,
                intake,
            },
            MessageKind::SantiSystem,
        )
    }

    fn append_message_with_kind(
        &self,
        draft: Draft<'_>,
        kind: MessageKind,
    ) -> Result<AppendedMessage, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let message_id = prefixed_id("msg");
        let now = timestamp_now();
        let content_json =
            serde_json::to_string(&draft.content).map_err(|error| error.to_string())?;
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
                draft.actor.encode(),
                draft.id,
                kind.encode(),
                content_json,
                draft.state.encode(),
                draft.intake.is_request() as i64,
                now
            ],
        )
        .map_err(|error| error.to_string())?;
        Database::new(&tx).append_entry_in_tx(
            draft.strand,
            StrandTargetType::Message,
            &message_id,
        )?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(AppendedMessage {
            strand_message: Database::new(&conn)
                .message_by_id(&message_id)?
                .ok_or_else(|| "created message missing".to_string())?,
        })
    }
}
