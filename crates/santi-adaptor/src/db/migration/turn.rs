use rusqlite::Connection;

pub fn migrate_v29_to_v30(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS turn_outbox (
            seq INTEGER PRIMARY KEY AUTOINCREMENT,
            id TEXT NOT NULL UNIQUE,
            turn_id TEXT NOT NULL UNIQUE,
            payload TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_turn_outbox_seq
        ON turn_outbox(seq);
        "#,
    )
    .map_err(|error| error.to_string())
}
