use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use rusqlite::{Connection, params};

use crate::store::{DEFAULT_SOUL_ID, SCHEMA_VERSION, SantiStore, schema::SCHEMA};
use crate::{prefixed_id, timestamp_now};

/// Read a DB's `user_version` WITHOUT opening the store (which would migrate
/// and, on a version mismatch, WIPE). `Ok(None)` when the file does not exist
/// yet (a fresh instance). Read-only: safe to run against a live service (WAL)
/// or a stopped one. Used by the offline pre-check `santi doctor`.
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

/// The default soul's memory file, given a runtime root:
/// `<runtime_root>/souls/<soul_id>/memory/MEMORY.md`. A free function so offline
/// ops can compute the path without a `SantiService`; the running service's
/// `soul_memory_file` (service/tools.rs) delegates here to stay in lockstep.
pub fn soul_memory_file(runtime_root: impl AsRef<Path>, soul_id: &str) -> std::path::PathBuf {
    runtime_root
        .as_ref()
        .join("souls")
        .join(soul_id)
        .join("memory")
        .join(crate::workspace_uri::MEMORY_FILE)
}

fn migrate_v21_to_v22(conn: &Connection) -> Result<(), String> {
    // v22 adds enqueue provenance to the inbox. These fields are nullable by
    // design so existing v21 rows keep their original content/created_at
    // semantics while new ingress can attach bounded source metadata.
    add_column_if_missing(conn, "strand_inbox", "source_type", "TEXT")?;
    add_column_if_missing(conn, "strand_inbox", "source_ref", "TEXT")?;
    add_column_if_missing(conn, "strand_inbox", "source_metadata", "TEXT")?;
    Ok(())
}

fn migrate_v22_to_v23(conn: &Connection) -> Result<(), String> {
    if table_exists(conn, "compacts")? {
        add_column_if_missing(conn, "compacts", "created_at", "TEXT")?;
        add_column_if_missing(conn, "compacts", "metadata", "TEXT")?;
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

#[derive(Debug)]
struct V23Block {
    id: String,
    strand_id: String,
    status: String,
    reason_code: String,
    reason_text: String,
    provider: Option<String>,
    model: Option<String>,
    budget_source: Option<String>,
    budget_bytes: Option<i64>,
    input_items: Option<i64>,
    input_bytes: Option<i64>,
    instructions_bytes: Option<i64>,
    tools_bytes: Option<i64>,
    total_bytes: Option<i64>,
    observed_turn_id: Option<String>,
    observed_at_seq: Option<i64>,
    metadata: Option<String>,
    created_at: String,
    updated_at: String,
    cleared_at: Option<String>,
    cleared_by: Option<String>,
}

fn migrate_v23_to_v24(conn: &Connection) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    tx.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS error_incidents (
            id TEXT PRIMARY KEY,
            incident_key TEXT NOT NULL,
            code TEXT NOT NULL,
            status TEXT NOT NULL CHECK (status IN ('active', 'resolved')),
            category TEXT NOT NULL,
            severity TEXT NOT NULL,
            retry TEXT NOT NULL,
            exposure TEXT NOT NULL,
            scope_kind TEXT NOT NULL,
            scope_id TEXT NOT NULL,
            source_component TEXT NOT NULL,
            source_operation TEXT NOT NULL,
            latest_source_component TEXT NOT NULL,
            latest_source_operation TEXT NOT NULL,
            message TEXT NOT NULL,
            latest_message TEXT NOT NULL,
            context TEXT NOT NULL,
            latest_context TEXT NOT NULL,
            occurrence_count INTEGER NOT NULL CHECK (occurrence_count > 0),
            revision INTEGER NOT NULL CHECK (revision > 0),
            first_seen_at TEXT NOT NULL,
            last_seen_at TEXT NOT NULL,
            resolved_at TEXT,
            resolved_by TEXT
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_error_incidents_active_key
        ON error_incidents(incident_key)
        WHERE status = 'active';
        CREATE INDEX IF NOT EXISTS idx_error_incidents_scope_time
        ON error_incidents(scope_kind, scope_id, first_seen_at);

        CREATE TABLE IF NOT EXISTS error_transitions (
            id TEXT PRIMARY KEY,
            incident_id TEXT NOT NULL,
            revision INTEGER NOT NULL,
            kind TEXT NOT NULL CHECK (kind IN ('opened', 'resolved')),
            payload TEXT NOT NULL,
            created_at TEXT NOT NULL,
            delivered_at TEXT,
            UNIQUE (incident_id, revision)
        );
        CREATE INDEX IF NOT EXISTS idx_error_transitions_pending
        ON error_transitions(created_at, id)
        WHERE delivered_at IS NULL;
        "#,
    )
    .map_err(|error| error.to_string())?;

    if table_exists(&tx, "strand_blocks")? {
        let mut stmt = tx
            .prepare(
                r#"
                SELECT id, strand_id, status, reason_code, reason_text, provider,
                       model, budget_source, budget_bytes, input_items, input_bytes,
                       instructions_bytes, tools_bytes, total_bytes, observed_turn_id,
                       observed_at_seq, metadata, created_at, updated_at, cleared_at,
                       cleared_by
                FROM strand_blocks
                ORDER BY created_at ASC, id ASC
                "#,
            )
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok(V23Block {
                    id: row.get(0)?,
                    strand_id: row.get(1)?,
                    status: row.get(2)?,
                    reason_code: row.get(3)?,
                    reason_text: row.get(4)?,
                    provider: row.get(5)?,
                    model: row.get(6)?,
                    budget_source: row.get(7)?,
                    budget_bytes: row.get(8)?,
                    input_items: row.get(9)?,
                    input_bytes: row.get(10)?,
                    instructions_bytes: row.get(11)?,
                    tools_bytes: row.get(12)?,
                    total_bytes: row.get(13)?,
                    observed_turn_id: row.get(14)?,
                    observed_at_seq: row.get(15)?,
                    metadata: row.get(16)?,
                    created_at: row.get(17)?,
                    updated_at: row.get(18)?,
                    cleared_at: row.get(19)?,
                    cleared_by: row.get(20)?,
                })
            })
            .map_err(|error| error.to_string())?;
        let mut blocks = Vec::new();
        for row in rows {
            blocks.push(row.map_err(|error| error.to_string())?);
        }
        drop(stmt);

        let has_rejections = table_exists(&tx, "rejected_deliveries")?;
        for block in blocks {
            let occurrences = if has_rejections {
                tx.query_row(
                    "SELECT COUNT(*) FROM rejected_deliveries WHERE block_id = ?1",
                    params![block.id],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|error| error.to_string())?
                .max(1)
            } else {
                1
            };
            let metadata = block
                .metadata
                .as_deref()
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok());
            let operation = metadata
                .as_ref()
                .and_then(|value| value.get("phase"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or(&block.reason_code)
                .to_string();
            let context = serde_json::json!({
                "schema": "santi.error.context_budget.v1",
                "reason": block.reason_code,
                "provider": block.provider,
                "model": block.model,
                "budget": {
                    "source": block.budget_source,
                    "input_bytes": block.budget_bytes,
                },
                "estimate": {
                    "input_items": block.input_items,
                    "input_bytes": block.input_bytes,
                    "instructions_bytes": block.instructions_bytes,
                    "tools_bytes": block.tools_bytes,
                    "total_bytes": block.total_bytes,
                },
                "observed_turn_id": block.observed_turn_id,
                "observed_at_seq": block.observed_at_seq,
                "details": metadata,
                "migration": "v23_to_v24",
            });
            let status = if block.status == "active" {
                "active"
            } else {
                "resolved"
            };
            let revision = if status == "active" { 1 } else { 2 };
            let incident_id = block
                .id
                .strip_prefix("blk_")
                .map(|suffix| format!("inc_{suffix}"))
                .unwrap_or_else(|| prefixed_id("inc"));
            tx.execute(
                r#"
                INSERT INTO error_incidents (
                  id, incident_key, code, status, category, severity, retry, exposure,
                  scope_kind, scope_id, source_component, source_operation,
                  latest_source_component, latest_source_operation, message,
                  latest_message, context, latest_context, occurrence_count, revision,
                  first_seen_at, last_seen_at, resolved_at, resolved_by
                ) VALUES (
                  ?1, ?2, 'context.budget.exceeded', ?3, 'resource_exhausted',
                  'error', 'after_resolution', ?4, 'strand', ?5, 'santi-core', ?6,
                  'santi-core', ?6, ?7, ?7, ?8, ?8, ?9, ?10, ?11, ?12, ?13, ?14
                )
                "#,
                params![
                    incident_id,
                    format!("context.budget.exceeded:strand:{}", block.strand_id),
                    status,
                    serde_json::to_string(&santi_error::ErrorExposure::CALLER_AND_OPERATOR)
                        .map_err(|error| error.to_string())?,
                    block.strand_id,
                    operation,
                    block.reason_text,
                    serde_json::to_string(&context).map_err(|error| error.to_string())?,
                    occurrences,
                    revision,
                    block.created_at,
                    block.updated_at,
                    block.cleared_at,
                    block.cleared_by,
                ],
            )
            .map_err(|error| error.to_string())?;
        }
    }

    tx.execute_batch(
        r#"
        DROP TABLE IF EXISTS rejected_deliveries;
        DROP TABLE IF EXISTS strand_blocks;
        "#,
    )
    .map_err(|error| error.to_string())?;
    tx.commit().map_err(|error| error.to_string())
}

fn table_exists(conn: &Connection, table: &str) -> Result<bool, String> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            params![table],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    Ok(count > 0)
}

fn add_column_if_missing(
    conn: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<(), String> {
    let count: i64 = conn
        .query_row(
            &format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name = ?1"),
            params![column],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if count == 0 {
        conn.execute(
            &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
            [],
        )
        .map_err(|error| error.to_string())?;
    }
    Ok(())
}

impl SantiStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let conn = Connection::open(path).map_err(|error| error.to_string())?;
        // Wait (don't fail) when another connection holds the write lock — the
        // offline `im reply` egress writes the same file while the server runs.
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
        if version == 21 && SCHEMA_VERSION == 25 {
            // v21 -> v22 is additive: PR #47 only adds bounded inbox-source
            // provenance columns. Migrate it in place so live ingress topology
            // (notably `webhooks` / the secretary subscription) cannot be
            // silently severed by this schema bump.
            migrate_v21_to_v22(&conn)?;
            migrate_v22_to_v23(&conn)?;
            migrate_v23_to_v24(&conn)?;
            super::receipt_migration::migrate_v24_to_v25(&conn)?;
        } else if version == 22 && SCHEMA_VERSION == 25 {
            migrate_v22_to_v23(&conn)?;
            migrate_v23_to_v24(&conn)?;
            super::receipt_migration::migrate_v24_to_v25(&conn)?;
        } else if version == 23 && SCHEMA_VERSION == 25 {
            migrate_v23_to_v24(&conn)?;
            super::receipt_migration::migrate_v24_to_v25(&conn)?;
        } else if version == 24 && SCHEMA_VERSION == 25 {
            super::receipt_migration::migrate_v24_to_v25(&conn)?;
        } else if version != SCHEMA_VERSION {
            // Fallback beta policy for unrecognized schema jumps: drop the
            // current runtime workspace and rebuild it. This must keep shrinking
            // as more runtime topology/evidence graduates into migrated tiers.
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
}
