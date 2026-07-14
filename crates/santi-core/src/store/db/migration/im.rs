use rusqlite::{Connection, params};

pub(in crate::store::db) fn migrate_v26_to_v27(conn: &Connection) -> Result<(), String> {
    if !table_exists(conn, "im_inbox")? {
        return Ok(());
    }
    add_column_if_missing(conn, "im_inbox", "turn_id", "TEXT")?;
    add_column_if_missing(conn, "im_inbox", "message_id", "TEXT")?;
    add_column_if_missing(conn, "im_inbox", "delivery_mode", "TEXT")?;
    conn.execute_batch(
        r#"
        CREATE UNIQUE INDEX IF NOT EXISTS idx_im_inbox_turn
        ON im_inbox(turn_id)
        WHERE turn_id IS NOT NULL;
        "#,
    )
    .map_err(|error| error.to_string())
}

fn table_exists(conn: &Connection, table: &str) -> Result<bool, String> {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        params![table],
        |row| row.get::<_, i64>(0),
    )
    .map(|count| count > 0)
    .map_err(|error| error.to_string())
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
