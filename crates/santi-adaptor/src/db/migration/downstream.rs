use rusqlite::{Connection, params};

pub fn migrate_v30_to_v31(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS downstreams (
            id TEXT PRIMARY KEY,
            label_prefix TEXT NOT NULL,
            credential_env TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        "#,
    )
    .map_err(|error| error.to_string())
}

pub fn migrate_v31_to_v32(conn: &Connection) -> Result<(), String> {
    let digest_ready = column_exists(conn, "downstreams", "credential_sha256")?;
    let outbox_ready = column_exists(conn, "turn_outbox", "external_label")?;
    if digest_ready && outbox_ready {
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS downstream_ingest (
                downstream_id TEXT NOT NULL,
                request_id TEXT NOT NULL,
                request_sha256 TEXT NOT NULL,
                strand_id TEXT NOT NULL,
                inbox_id TEXT NOT NULL,
                created_at TEXT NOT NULL,
                PRIMARY KEY (downstream_id, request_id)
            );
            CREATE INDEX IF NOT EXISTS idx_downstream_ingest_receipt
            ON downstream_ingest(inbox_id);
            CREATE INDEX IF NOT EXISTS idx_turn_outbox_label_seq
            ON turn_outbox(external_label, seq);
            "#,
        )
        .map_err(|error| error.to_string())?;
        return Ok(());
    }
    if digest_ready || outbox_ready {
        return Err("downstream v32 schema is only partially present".to_string());
    }
    conn.execute_batch(
        r#"
        DROP TABLE downstreams;
        CREATE TABLE downstreams (
            id TEXT PRIMARY KEY,
            label_prefix TEXT NOT NULL UNIQUE,
            credential_sha256 TEXT NOT NULL UNIQUE,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE TABLE downstream_ingest (
            downstream_id TEXT NOT NULL,
            request_id TEXT NOT NULL,
            request_sha256 TEXT NOT NULL,
            strand_id TEXT NOT NULL,
            inbox_id TEXT NOT NULL,
            created_at TEXT NOT NULL,
            PRIMARY KEY (downstream_id, request_id)
        );
        CREATE INDEX idx_downstream_ingest_receipt
        ON downstream_ingest(inbox_id);

        ALTER TABLE turn_outbox RENAME TO turn_outbox_v31;
        DROP INDEX IF EXISTS idx_turn_outbox_seq;
        CREATE TABLE turn_outbox (
            seq INTEGER PRIMARY KEY AUTOINCREMENT,
            id TEXT NOT NULL UNIQUE,
            turn_id TEXT NOT NULL UNIQUE,
            external_label TEXT NOT NULL,
            payload TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        INSERT INTO turn_outbox (
          seq, id, turn_id, external_label, payload, created_at
        )
        SELECT seq, id, turn_id, json_extract(payload, '$.external_label'), payload, created_at
        FROM turn_outbox_v31;
        DROP TABLE turn_outbox_v31;
        CREATE INDEX idx_turn_outbox_seq ON turn_outbox(seq);
        CREATE INDEX idx_turn_outbox_label_seq
        ON turn_outbox(external_label, seq);
        "#,
    )
    .map_err(|error| error.to_string())
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool, String> {
    conn.query_row(
        &format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name = ?1"),
        params![column],
        |row| row.get::<_, i64>(0),
    )
    .map(|count| count > 0)
    .map_err(|error| error.to_string())
}
