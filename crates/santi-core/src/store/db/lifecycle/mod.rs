use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use rusqlite::{Connection, params};

use crate::store::{DEFAULT_SOUL_ID, SCHEMA_VERSION, SantiStore, schema::SCHEMA};
use crate::timestamp_now;

mod migrate;
use migrate::*;

struct Schema<'a>(&'a Connection);

impl Schema<'_> {
    fn table_exists(&self, table: &str) -> Result<bool, String> {
        let count: i64 = self
            .0
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                params![table],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        Ok(count > 0)
    }

    fn add_column(&self, table: &str, column: &str, definition: &str) -> Result<(), String> {
        let count: i64 = self
            .0
            .query_row(
                &format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name = ?1"),
                params![column],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if count == 0 {
            self.0
                .execute(
                    &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
                    [],
                )
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }
}

pub fn read_schema_version(path: impl AsRef<Path>) -> Result<Option<u32>, String> {
    if !path.as_ref().exists() {
        return Ok(None);
    }
    let conn = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .map_err(|error| error.to_string())?;
    conn.query_row("PRAGMA user_version", [], |row| row.get::<_, u32>(0))
        .map(Some)
        .map_err(|error| error.to_string())
}

pub fn soul_memory_file(runtime_root: impl AsRef<Path>, soul_id: &str) -> std::path::PathBuf {
    runtime_root
        .as_ref()
        .join("souls")
        .join(soul_id)
        .join("memory")
        .join(crate::workspace::MEMORY_FILE)
}

fn migrate_v21_to_v22(conn: &Connection) -> Result<(), String> {
    let schema = Schema(conn);
    schema.add_column("strand_inbox", "source_type", "TEXT")?;
    schema.add_column("strand_inbox", "source_ref", "TEXT")?;
    schema.add_column("strand_inbox", "source_metadata", "TEXT")?;
    Ok(())
}

fn migrate_v22_to_v23(conn: &Connection) -> Result<(), String> {
    let schema = Schema(conn);
    if schema.table_exists("compacts")? {
        schema.add_column("compacts", "created_at", "TEXT")?;
        schema.add_column("compacts", "metadata", "TEXT")?;
    }
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS strand_blocks (
            id TEXT PRIMARY KEY,
            strand_id TEXT NOT NULL,
            kind TEXT NOT NULL CHECK (kind IN ('context_over_budget')),
            status TEXT NOT NULL CHECK (status IN ('active', 'cleared')),
            reason_code TEXT NOT NULL,
            reason_text TEXT NOT NULL,
            provider TEXT,
            model TEXT,
            budget_source TEXT,
            budget_bytes INTEGER,
            input_items INTEGER,
            input_bytes INTEGER,
            instructions_bytes INTEGER,
            tools_bytes INTEGER,
            total_bytes INTEGER,
            observed_turn_id TEXT,
            observed_at_seq INTEGER,
            metadata TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            cleared_at TEXT,
            cleared_by TEXT
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_strand_blocks_active_context
        ON strand_blocks(strand_id, kind)
        WHERE status = 'active';
        CREATE INDEX IF NOT EXISTS idx_strand_blocks_strand_created_at
        ON strand_blocks(strand_id, created_at);

        CREATE TABLE IF NOT EXISTS rejected_deliveries (
            id TEXT PRIMARY KEY,
            strand_id TEXT,
            block_id TEXT,
            source_type TEXT,
            source_ref TEXT,
            source_metadata TEXT,
            message_kind TEXT,
            content_sha256 TEXT NOT NULL,
            content_bytes INTEGER NOT NULL,
            content_excerpt TEXT NOT NULL,
            reason_code TEXT NOT NULL,
            reason_text TEXT NOT NULL,
            received_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_rejected_deliveries_strand_time
        ON rejected_deliveries(strand_id, received_at);
        "#,
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

impl SantiStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let conn = Connection::open(path).map_err(|error| error.to_string())?;
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|error| error.to_string())?;
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
        if version == 21 && SCHEMA_VERSION == 28 {
            migrate_v21_to_v22(&conn)?;
            migrate_v22_to_v23(&conn)?;
            migrate_v23_to_v24(&conn)?;
            super::migration::receipt::migrate_v24_to_v25(&conn)?;
            super::migration::effect::migrate_v25_to_v26(&conn)?;
            super::migration::im::migrate_v26_to_v27(&conn)?;
            super::migration::window::migrate_v27_to_v28(&conn)?;
        } else if version == 22 && SCHEMA_VERSION == 28 {
            migrate_v22_to_v23(&conn)?;
            migrate_v23_to_v24(&conn)?;
            super::migration::receipt::migrate_v24_to_v25(&conn)?;
            super::migration::effect::migrate_v25_to_v26(&conn)?;
            super::migration::im::migrate_v26_to_v27(&conn)?;
            super::migration::window::migrate_v27_to_v28(&conn)?;
        } else if version == 23 && SCHEMA_VERSION == 28 {
            migrate_v23_to_v24(&conn)?;
            super::migration::receipt::migrate_v24_to_v25(&conn)?;
            super::migration::effect::migrate_v25_to_v26(&conn)?;
            super::migration::im::migrate_v26_to_v27(&conn)?;
            super::migration::window::migrate_v27_to_v28(&conn)?;
        } else if version == 24 && SCHEMA_VERSION == 28 {
            super::migration::receipt::migrate_v24_to_v25(&conn)?;
            super::migration::effect::migrate_v25_to_v26(&conn)?;
            super::migration::im::migrate_v26_to_v27(&conn)?;
            super::migration::window::migrate_v27_to_v28(&conn)?;
        } else if version == 25 && SCHEMA_VERSION == 28 {
            super::migration::effect::migrate_v25_to_v26(&conn)?;
            super::migration::im::migrate_v26_to_v27(&conn)?;
            super::migration::window::migrate_v27_to_v28(&conn)?;
        } else if version == 26 && SCHEMA_VERSION == 28 {
            super::migration::im::migrate_v26_to_v27(&conn)?;
            super::migration::window::migrate_v27_to_v28(&conn)?;
        } else if version == 27 && SCHEMA_VERSION == 28 {
            super::migration::window::migrate_v27_to_v28(&conn)?;
        } else if version != SCHEMA_VERSION {
            conn.execute_batch(
                r#"
                DROP TABLE IF EXISTS provider_replay_material;
                DROP TABLE IF EXISTS response_stream_deltas;
                DROP TABLE IF EXISTS response_runs;
                DROP TABLE IF EXISTS message_text_contents;
                DROP TABLE IF EXISTS conversations;
                DROP TABLE IF EXISTS r_strand_entries;
                DROP TABLE IF EXISTS strand_inbox;
                DROP TABLE IF EXISTS receipt_transitions;
                DROP TABLE IF EXISTS inbox_receipts;
                DROP TABLE IF EXISTS effect_transitions;
                DROP TABLE IF EXISTS window_messages;
                DROP TABLE IF EXISTS im_inbox;
                DROP TABLE IF EXISTS im_participants;
                DROP TABLE IF EXISTS compacts;
                DROP TABLE IF EXISTS error_transitions;
                DROP TABLE IF EXISTS error_incidents;
                DROP TABLE IF EXISTS strand_blocks;
                DROP TABLE IF EXISTS rejected_deliveries;
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
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn seed_defaults(&self) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        let now = timestamp_now();
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
}
