use super::*;

#[test]
fn schema_retires_integrated_im_without_touching_sidecar() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db = temp.path().join("santi.sqlite");
    drop(SantiStore::open(&db).expect("open current store"));
    let conn = Connection::open(&db).expect("open sqlite");
    conn.execute_batch(
        r#"
        CREATE TABLE reply_outbox (
            id TEXT PRIMARY KEY,
            turn_id TEXT NOT NULL UNIQUE,
            payload TEXT NOT NULL,
            created_at TEXT NOT NULL,
            delivered_at TEXT
        );
        CREATE TABLE window_messages (
            participant_id TEXT NOT NULL,
            client_message_id TEXT NOT NULL,
            inbox_id TEXT NOT NULL,
            message_id TEXT NOT NULL,
            content_hash TEXT NOT NULL,
            received_at TEXT NOT NULL,
            PRIMARY KEY (participant_id, client_message_id)
        );
        INSERT INTO reply_outbox VALUES (
            'reply_1', 'turn_1', '{}', '2026-07-22T00:00:00Z', NULL
        );
        PRAGMA user_version = 32;
        "#,
    )
    .expect("create v32 reply outbox");
    drop(conn);
    let mut sidecar = db.as_os_str().to_owned();
    sidecar.push(".im");
    let sidecar = std::path::PathBuf::from(sidecar);
    std::fs::write(&sidecar, "preserve").expect("write sidecar marker");

    drop(SantiStore::open(&db).expect("migrate v32 to v33"));
    assert_eq!(
        santi_core::read_schema_version(&db).expect("schema version"),
        Some(santi_core::SCHEMA_VERSION)
    );
    let conn = Connection::open(db).expect("reopen sqlite");
    let retired: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN ('reply_outbox', 'window_messages')",
            [],
            |row| row.get(0),
        )
        .expect("retired table lookup");
    assert_eq!(retired, 0);
    assert_eq!(
        std::fs::read_to_string(sidecar).expect("read sidecar marker"),
        "preserve"
    );
}
