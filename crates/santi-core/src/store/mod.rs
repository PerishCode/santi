use std::sync::{Arc, Mutex};

use rusqlite::{Connection, params};

use crate::{Fault, now, strand::Strand, tag, turn::Turn};

mod archive;
mod assembly;
pub(crate) mod budget;
mod compact;
pub(crate) mod errors;
mod ledger;
use ledger::{db, effects, rows, span};
mod lifecycle;
mod runtime;
mod turns;

pub(crate) use budget::{Ingress, Launch, Notice, Offered, Replay};
pub(crate) use compact::Collapse;
pub use db::version;
use db::*;
pub(crate) use effects::Settlement;
pub(crate) use ledger::{
    Attention as JobAttention, Entry as JobEntry, Grant as JobGrant, Prepared as JobPrepared,
    Record as JobRecord,
};
use rows::{Decode, collected};
pub use runtime::Invocation;
pub use turns::Completion;

use crate::{message, strand};
use santi_adaptor::SYSTEM;
pub use santi_adaptor::VERSION;
pub const GENESIS: &str = "soul_default";
const GATE: i64 = 500;

pub fn memoir(runtime: impl AsRef<std::path::Path>, soul: &str) -> std::path::PathBuf {
    runtime
        .as_ref()
        .join("souls")
        .join(soul)
        .join("memory")
        .join(crate::workspace::MEMORY)
}

#[derive(Clone)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
}

#[derive(Debug, Clone)]
pub struct Penned {
    pub message: message::Placed,
}

pub struct Draft<'a> {
    pub strand: &'a str,
    pub actor: message::Role,
    pub id: &'a str,
    pub content: message::Content,
    pub state: message::State,
    pub intake: message::Intake,
}

#[derive(Debug, Clone)]
pub struct Begun {
    pub turn: Turn,
    pub drained: Vec<message::Placed>,
}

pub(crate) enum Opened {
    Started(Begun),
    Running(Turn),
    Idle,
    Held(Fault),
}

pub(crate) struct Misfire<'a> {
    pub provider: &'a str,
    pub model: &'a str,
    pub stage: &'a str,
    pub operation: &'a str,
    pub round: usize,
    pub detail: &'a str,
}

pub(crate) struct Stumble<'a> {
    pub operation: &'a str,
    pub detail: &'a str,
}

impl Store {
    pub fn sink(&self) -> plumb::trace::Sink {
        plumb::trace::Sink::from(Arc::new(archive::Archive::open(self.conn.clone())))
    }

    pub fn trail(&self, key: &str, value: &str) -> Result<Vec<crate::trace::Record>, String> {
        let conn = self.conn.lock().unwrap();
        Database::new(&conn).traced(key, value)
    }

    pub fn genesis(&self) -> &'static str {
        GENESIS
    }

    pub fn system(&self) -> &'static str {
        SYSTEM
    }

    pub fn strands(&self) -> Result<Vec<Strand>, String> {
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
        collected(rows)
    }

    pub fn weave(&self) -> Result<Strand, String> {
        let conn = self.conn.lock().unwrap();
        let strand = tag("ss");
        let now = now();
        conn.execute(
            r#"
            INSERT INTO strands (
              id, soul_id, external_label, strand_memory, provider_state, next_seq,
              last_seen_strand_seq, parent_strand_id, fork_point, created_at, updated_at
            )
            VALUES (?1, ?2, NULL, '', NULL, 1, 0, NULL, NULL, ?3, ?3)
            "#,
            params![strand, GENESIS, now],
        )
        .map_err(|error| error.to_string())?;
        Database::new(&conn)
            .strand(&strand)?
            .ok_or_else(|| "created strand missing".to_string())
    }

    pub fn messages(&self, strand: &str) -> Result<Vec<message::Placed>, String> {
        let conn = self.conn.lock().unwrap();
        Database::new(&conn).messages(strand)
    }

    pub fn snapshot(&self, strand: &str) -> Result<Option<crate::stream::Snapshot>, String> {
        let conn = self.conn.lock().unwrap();
        let database = Database::new(&conn);
        let Some(strand) = database.strand(strand)? else {
            return Ok(None);
        };
        Ok(Some(crate::stream::Snapshot {
            messages: database.messages(&strand.id)?,
            events: database.events(&strand.id)?,
            turns: database.turns(&strand.id)?,
            thinking: database.thinking(&strand.id)?,
            calls: database.calls(&strand.id)?,
            results: database.results(&strand.id)?,
            compacts: database.compacts(&strand.id)?,
            effects: database.effects(&strand.id)?,
            errors: database.incidents("strand", &strand.id, 100)?,
            strand,
        }))
    }

    pub fn pen(&self, draft: Draft<'_>) -> Result<Penned, String> {
        self.compose(draft, message::Kind::Text)
    }

    pub fn inscribe(
        &self,
        strand: &str,
        content: message::Content,
        intake: message::Intake,
    ) -> Result<Penned, String> {
        self.compose(
            Draft {
                strand,
                actor: message::Role::System,
                id: SYSTEM,
                content,
                state: message::State::Fixed,
                intake,
            },
            message::Kind::SantiSystem,
        )
    }

    fn compose(&self, draft: Draft<'_>, kind: message::Kind) -> Result<Penned, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let message = tag("msg");
        let now = now();
        let blob = serde_json::to_string(&draft.content).map_err(|error| error.to_string())?;
        tx.execute(
            r#"
            INSERT INTO messages (
              id, actor_type, actor_id, message_kind, content, state, version, is_request,
              deleted_at, created_at, updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7, NULL, ?8, ?8)
            "#,
            params![
                message,
                draft.actor.encode(),
                draft.id,
                kind.encode(),
                blob,
                draft.state.encode(),
                draft.intake.is_request() as i64,
                now
            ],
        )
        .map_err(|error| error.to_string())?;
        Database::new(&tx).entered(draft.strand, strand::Target::Message, &message)?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(Penned {
            message: Database::new(&conn)
                .message(&message)?
                .ok_or_else(|| "created message missing".to_string())?,
        })
    }
}
