use rusqlite::Connection;

pub fn migrate_v32_to_v33(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        DROP TABLE IF EXISTS reply_outbox;
        DROP TABLE IF EXISTS window_messages;
        "#,
    )
    .map_err(|error| error.to_string())
}
