use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use rusqlite::{Connection, OptionalExtension, params};

use crate::{
    ActorType, MessageContent, MessageIntake, MessageKind, MessageState, Session, SessionMessage,
    SessionSummary, SoulSession, SoulSessionEntry, SoulSessionTargetType, Turn, prefixed_id,
    timestamp_now,
};

mod assembly;
mod db;
mod rows;
mod runtime;
mod schema;

use db::*;
use rows::{
    actor_type_db, collect_rows, map_session_summary_row, map_webhook_row, message_state_db,
};
use schema::SCHEMA;

const SANTI_SCHEMA_VERSION: u32 = 13;
const DEFAULT_ACCOUNT_ID: &str = "account_local";
const DEFAULT_SOUL_ID: &str = "soul_default";
const SANTI_SYSTEM_ACTOR_ID: &str = "santi";

#[derive(Clone)]
pub struct SantiStore {
    conn: Arc<Mutex<Connection>>,
}

#[derive(Debug, Clone)]
pub struct AppendedMessage {
    pub session_message: SessionMessage,
}

#[derive(Debug, Clone)]
pub struct AcquiredSoulSession {
    pub soul_session: SoulSession,
}

#[derive(Debug, Clone)]
pub struct StartedTurn {
    pub turn: Turn,
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
                DROP TABLE IF EXISTS r_soul_session_messages;
                DROP TABLE IF EXISTS compacts;
                DROP TABLE IF EXISTS thinking_spans;
                DROP TABLE IF EXISTS tool_results;
                DROP TABLE IF EXISTS tool_calls;
                DROP TABLE IF EXISTS turns;
                DROP TABLE IF EXISTS soul_sessions;
                DROP TABLE IF EXISTS session_effects;
                DROP TABLE IF EXISTS message_events;
                DROP TABLE IF EXISTS r_session_messages;
                DROP TABLE IF EXISTS messages;
                DROP TABLE IF EXISTS webhooks;
                DROP TABLE IF EXISTS session_profiles;
                DROP TABLE IF EXISTS sessions;
                DROP TABLE IF EXISTS soul_profiles;
                DROP TABLE IF EXISTS souls;
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
        conn.execute(
            r#"
            INSERT OR IGNORE INTO accounts (id, name, created_at, updated_at)
            VALUES (?1, 'Local Account', ?2, ?2)
            "#,
            params![DEFAULT_ACCOUNT_ID, now],
        )
        .map_err(|error| error.to_string())?;
        conn.execute(
            r#"
            INSERT OR IGNORE INTO souls (id, memory, created_at, updated_at)
            VALUES (?1, '', ?2, ?2)
            "#,
            params![DEFAULT_SOUL_ID, now],
        )
        .map_err(|error| error.to_string())?;
        conn.execute(
            r#"
            INSERT OR IGNORE INTO soul_profiles (
              soul_id, soul_name, nickname, avatar_ref, avatar_seed, desc, created_at, updated_at
            )
            VALUES (?1, 'Liberte', 'Santi', NULL, ?1, NULL, ?2, ?2)
            "#,
            params![DEFAULT_SOUL_ID, now],
        )
        .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn default_account_id(&self) -> &'static str {
        DEFAULT_ACCOUNT_ID
    }

    pub fn default_soul_id(&self) -> &'static str {
        DEFAULT_SOUL_ID
    }

    pub fn list_sessions(&self) -> Result<Vec<SessionSummary>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                r#"
                SELECT
                  s.id, s.parent_session_id, s.fork_point, s.created_at, s.updated_at,
                  p.session_id, p.title, p.desc, p.created_at, p.updated_at
                FROM sessions s
                JOIN session_profiles p ON p.session_id = s.id
                ORDER BY s.updated_at DESC, s.id DESC
                "#,
            )
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map([], map_session_summary_row)
            .map_err(|error| error.to_string())?;
        collect_rows(rows)
    }

    pub fn create_session(&self) -> Result<SessionSummary, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let session_id = prefixed_id("sess");
        let now = timestamp_now();
        tx.execute(
            r#"
            INSERT INTO sessions (id, parent_session_id, fork_point, created_at, updated_at)
            VALUES (?1, NULL, NULL, ?2, ?2)
            "#,
            params![session_id, now],
        )
        .map_err(|error| error.to_string())?;
        tx.execute(
            r#"
            INSERT INTO session_profiles (session_id, title, desc, created_at, updated_at)
            VALUES (?1, NULL, NULL, ?2, ?2)
            "#,
            params![session_id, now],
        )
        .map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())?;
        session_summary_by_id(&conn, &session_id)?
            .ok_or_else(|| "created session missing".to_string())
    }

    pub fn session(&self, session_id: &str) -> Result<Option<Session>, String> {
        let conn = self.conn.lock().unwrap();
        session_by_id(&conn, session_id)
    }

    pub fn update_session_title(
        &self,
        session_id: &str,
        title: Option<String>,
    ) -> Result<Option<SessionSummary>, String> {
        let conn = self.conn.lock().unwrap();
        if session_by_id(&conn, session_id)?.is_none() {
            return Ok(None);
        }
        let now = timestamp_now();
        conn.execute(
            "UPDATE session_profiles SET title = ?2, updated_at = ?3 WHERE session_id = ?1",
            params![session_id, normalize_session_title(title), now],
        )
        .map_err(|error| error.to_string())?;
        conn.execute(
            "UPDATE sessions SET updated_at = ?2 WHERE id = ?1",
            params![session_id, now],
        )
        .map_err(|error| error.to_string())?;
        session_summary_by_id(&conn, session_id)
    }

    pub fn session_messages(&self, session_id: &str) -> Result<Vec<SessionMessage>, String> {
        let conn = self.conn.lock().unwrap();
        session_messages(&conn, session_id)
    }

    pub fn runtime_snapshot(
        &self,
        soul_id: &str,
        session_id: &str,
    ) -> Result<Option<crate::SessionRuntimeSnapshot>, String> {
        let conn = self.conn.lock().unwrap();
        let Some(session) = session_by_id(&conn, session_id)? else {
            return Ok(None);
        };
        let profile = session_profile_by_id(&conn, session_id)?
            .ok_or_else(|| "session profile missing".to_string())?;
        let soul_session = soul_session_by_pair(&conn, soul_id, session_id)?;
        let soul_profile = soul_profile_by_id(&conn, soul_id)?;
        Ok(Some(crate::SessionRuntimeSnapshot {
            session,
            profile,
            soul_session: soul_session.clone(),
            soul_profile,
            messages: session_messages(&conn, session_id)?,
            turns: if let Some(soul_session) = &soul_session {
                turns_for_soul_session(&conn, &soul_session.id)?
            } else {
                Vec::new()
            },
            thinking_spans: if let Some(soul_session) = &soul_session {
                soul_thinking_spans(&conn, &soul_session.id)?
            } else {
                Vec::new()
            },
            tool_calls: if let Some(soul_session) = &soul_session {
                soul_tool_calls(&conn, &soul_session.id)?
            } else {
                Vec::new()
            },
            tool_results: if let Some(soul_session) = &soul_session {
                soul_tool_results(&conn, &soul_session.id)?
            } else {
                Vec::new()
            },
            compacts: if let Some(soul_session) = &soul_session {
                compacts_for_soul_session(&conn, &soul_session.id)?
            } else {
                Vec::new()
            },
            effects: session_effects(&conn, session_id)?,
        }))
    }

    pub fn append_message(
        &self,
        session_id: &str,
        actor_type: ActorType,
        actor_id: &str,
        content: MessageContent,
        state: MessageState,
        intake: MessageIntake,
    ) -> Result<AppendedMessage, String> {
        self.append_message_with_kind(
            session_id,
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
        session_id: &str,
        content: MessageContent,
        intake: MessageIntake,
    ) -> Result<AppendedMessage, String> {
        self.append_message_with_kind(
            session_id,
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
        session_id: &str,
        actor_type: ActorType,
        actor_id: &str,
        message_kind: MessageKind,
        content: MessageContent,
        state: MessageState,
        intake: MessageIntake,
    ) -> Result<AppendedMessage, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        ensure_session(&tx, session_id)?;
        let message_id = prefixed_id("msg");
        let now = timestamp_now();
        let next_seq = next_session_seq(&tx, session_id)?;
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
        tx.execute(
            r#"
            INSERT INTO r_session_messages (session_id, message_id, session_seq, created_at)
            VALUES (?1, ?2, ?3, ?4)
            "#,
            params![session_id, message_id, next_seq, now],
        )
        .map_err(|error| error.to_string())?;
        let title = if actor_type == ActorType::Account && next_seq == 1 {
            session_title(&content)
        } else {
            None
        };
        if let Some(title) = title {
            tx.execute(
                "UPDATE session_profiles SET title = COALESCE(title, ?2), updated_at = ?3 WHERE session_id = ?1",
                params![session_id, title, now],
            )
            .map_err(|error| error.to_string())?;
            tx.execute(
                "UPDATE sessions SET updated_at = ?2 WHERE id = ?1",
                params![session_id, now],
            )
            .map_err(|error| error.to_string())?;
        } else {
            tx.execute(
                "UPDATE sessions SET updated_at = ?2 WHERE id = ?1",
                params![session_id, now],
            )
            .map_err(|error| error.to_string())?;
        }
        tx.commit().map_err(|error| error.to_string())?;
        Ok(AppendedMessage {
            session_message: message_by_id(&conn, &message_id)?
                .ok_or_else(|| "created message missing".to_string())?,
        })
    }

    pub fn acquire_soul_session(
        &self,
        soul_id: &str,
        session_id: &str,
    ) -> Result<AcquiredSoulSession, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        ensure_session(&tx, session_id)?;
        // Reject an unknown soul before inserting a pair: otherwise a bad
        // soul_id (now user-suppliable via `send`) would create an orphan
        // soul_session pointing at no soul, surfacing later as a cryptic
        // "soul_profile disappeared".
        if soul_profile_by_id(&tx, soul_id)?.is_none() {
            return Err(format!("unknown soul: {soul_id}"));
        }
        let now = timestamp_now();
        let existing = soul_session_by_pair(&tx, soul_id, session_id)?;
        let soul_session = if let Some(existing) = existing {
            existing
        } else {
            let soul_session_id = prefixed_id("ss");
            tx.execute(
                r#"
                INSERT INTO soul_sessions (
                  id, soul_id, session_id, session_memory, provider_state, next_seq,
                  last_seen_session_seq, parent_soul_session_id, fork_point, created_at, updated_at
                )
                VALUES (?1, ?2, ?3, '', NULL, 1, 0, NULL, NULL, ?4, ?4)
                "#,
                params![soul_session_id, soul_id, session_id, now],
            )
            .map_err(|error| error.to_string())?;
            soul_session_by_id(&tx, &soul_session_id)?
                .ok_or_else(|| "created soul_session missing".to_string())?
        };
        tx.commit().map_err(|error| error.to_string())?;
        Ok(AcquiredSoulSession { soul_session })
    }

    /// Find the session anchored to an opaque external label, or create one and
    /// bind it. The label is opaque to core (its meaning + any soul-scoping live
    /// in the adaptor); uniqueness is global, enforced by the partial index.
    pub fn find_or_create_session_by_label(&self, label: &str) -> Result<SessionSummary, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let existing: Option<String> = tx
            .query_row(
                "SELECT id FROM sessions WHERE external_label = ?1 LIMIT 1",
                params![label],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let session_id = if let Some(id) = existing {
            id
        } else {
            let session_id = prefixed_id("sess");
            let now = timestamp_now();
            tx.execute(
                r#"
                INSERT INTO sessions (id, parent_session_id, fork_point, external_label, created_at, updated_at)
                VALUES (?1, NULL, NULL, ?2, ?3, ?3)
                "#,
                params![session_id, label, now],
            )
            .map_err(|error| error.to_string())?;
            tx.execute(
                r#"
                INSERT INTO session_profiles (session_id, title, desc, created_at, updated_at)
                VALUES (?1, NULL, NULL, ?2, ?2)
                "#,
                params![session_id, now],
            )
            .map_err(|error| error.to_string())?;
            session_id
        };
        tx.commit().map_err(|error| error.to_string())?;
        session_summary_by_id(&conn, &session_id)?
            .ok_or_else(|| "labeled session missing".to_string())
    }

    /// Register a webhook subscription (API-managed). The secret itself is never
    /// stored — `secret_env` names the env var the adaptor reads at verify time.
    pub fn create_webhook(
        &self,
        name: &str,
        adaptor: &str,
        soul_id: &str,
        session_strategy: &str,
        secret_env: &str,
    ) -> Result<crate::WebhookSubscription, String> {
        let conn = self.conn.lock().unwrap();
        let now = timestamp_now();
        conn.execute(
            r#"
            INSERT INTO webhooks (
              name, adaptor, soul_id, session_strategy, secret_env, created_at, updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
            "#,
            params![name, adaptor, soul_id, session_strategy, secret_env, now],
        )
        .map_err(|error| error.to_string())?;
        webhook_by_name(&conn, name)?.ok_or_else(|| "created webhook missing".to_string())
    }

    pub fn list_webhooks(&self) -> Result<Vec<crate::WebhookSubscription>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                r#"
                SELECT name, adaptor, soul_id, session_strategy, secret_env, created_at, updated_at
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

    /// Create a new soul (an individual). Souls are API-managed, never config.
    pub fn create_soul(
        &self,
        soul_name: &str,
        nickname: &str,
        desc: Option<&str>,
    ) -> Result<crate::SoulProfile, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let soul_id = prefixed_id("soul");
        let now = timestamp_now();
        tx.execute(
            "INSERT INTO souls (id, memory, created_at, updated_at) VALUES (?1, '', ?2, ?2)",
            params![soul_id, now],
        )
        .map_err(|error| error.to_string())?;
        tx.execute(
            r#"
            INSERT INTO soul_profiles (
              soul_id, soul_name, nickname, avatar_ref, avatar_seed, desc, created_at, updated_at
            )
            VALUES (?1, ?2, ?3, NULL, ?1, ?4, ?5, ?5)
            "#,
            params![soul_id, soul_name, nickname, desc, now],
        )
        .map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())?;
        soul_profile_by_id(&conn, &soul_id)?.ok_or_else(|| "created soul missing".to_string())
    }

    pub fn list_souls(&self) -> Result<Vec<crate::SoulProfile>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                r#"
                SELECT soul_id, soul_name, nickname, avatar_ref, avatar_seed, desc,
                       created_at, updated_at
                FROM soul_profiles ORDER BY created_at ASC, soul_id ASC
                "#,
            )
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map([], rows::map_soul_profile_row)
            .map_err(|error| error.to_string())?;
        collect_rows(rows)
    }

    pub fn soul_profile(&self, soul_id: &str) -> Result<Option<crate::SoulProfile>, String> {
        let conn = self.conn.lock().unwrap();
        soul_profile_by_id(&conn, soul_id)
    }

    pub fn soul_session(&self, soul_session_id: &str) -> Result<Option<SoulSession>, String> {
        let conn = self.conn.lock().unwrap();
        soul_session_by_id(&conn, soul_session_id)
    }

    pub fn soul_id_for_soul_session(&self, soul_session_id: &str) -> Result<String, String> {
        self.soul_session(soul_session_id)?
            .map(|soul_session| soul_session.soul_id)
            .ok_or_else(|| "soul_session not found".to_string())
    }

    pub fn append_message_ref(
        &self,
        soul_session_id: &str,
        message_id: &str,
    ) -> Result<SoulSessionEntry, String> {
        self.append_soul_session_entry(soul_session_id, SoulSessionTargetType::Message, message_id)
    }

    pub fn start_turn(
        &self,
        soul_session_id: &str,
        trigger_ref: &str,
        input_through_session_seq: i64,
    ) -> Result<StartedTurn, String> {
        let conn = self.conn.lock().unwrap();
        let turn_id = prefixed_id("turn");
        let now = timestamp_now();
        conn.execute(
            r#"
            INSERT INTO turns (
              id, soul_session_id, trigger_type, trigger_ref, input_through_session_seq,
              base_soul_session_seq, end_soul_session_seq, status, error_text,
              created_at, updated_at, finished_at
            )
            SELECT ?1, id, 'session_send', ?3, ?4, next_seq - 1, NULL, 'running',
                   NULL, ?5, ?5, NULL
            FROM soul_sessions
            WHERE id = ?2
            "#,
            params![
                turn_id,
                soul_session_id,
                trigger_ref,
                input_through_session_seq,
                now
            ],
        )
        .map_err(|error| error.to_string())?;
        Ok(StartedTurn {
            turn: turn_by_id(&conn, &turn_id)?.ok_or_else(|| "created turn missing".to_string())?,
        })
    }
}

fn session_title(content: &MessageContent) -> Option<String> {
    let title = content
        .content_text()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    normalize_session_title(Some(title))
}

fn normalize_session_title(title: Option<String>) -> Option<String> {
    let title = title?;
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_string())
}
