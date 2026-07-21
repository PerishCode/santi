use rusqlite::Connection;

pub fn migrate_v27_to_v28(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS window_messages (
            participant_id TEXT NOT NULL,
            client_message_id TEXT NOT NULL,
            inbox_id TEXT NOT NULL UNIQUE,
            message_id TEXT NOT NULL UNIQUE,
            content_hash TEXT NOT NULL,
            cursor INTEGER,
            received_at TEXT NOT NULL,
            PRIMARY KEY (participant_id, client_message_id)
        );
        "#,
    )
    .map_err(|error| error.to_string())
}
