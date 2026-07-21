use rusqlite::Connection;

pub fn migrate_v28_to_v29(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS reply_outbox (
            id TEXT PRIMARY KEY,
            turn_id TEXT NOT NULL UNIQUE,
            payload TEXT NOT NULL,
            created_at TEXT NOT NULL,
            delivered_at TEXT
        );
        "#,
    )
    .map_err(|error| error.to_string())
}
