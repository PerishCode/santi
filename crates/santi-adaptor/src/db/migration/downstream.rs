use rusqlite::Connection;

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
