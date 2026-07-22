use super::*;

#[test]
fn schema_migrates_downstream_isolation_shape() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db = temp.path().join("santi.sqlite");
    let conn = Connection::open(&db).expect("open sqlite");
    conn.execute_batch(
        r#"
        CREATE TABLE downstreams (
            id TEXT PRIMARY KEY,
            label_prefix TEXT NOT NULL,
            credential_env TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        INSERT INTO downstreams VALUES (
            'stim', 'stim:', 'STIM_TOKEN',
            '2026-07-22T00:00:00Z', '2026-07-22T00:00:00Z'
        );
        CREATE TABLE turn_outbox (
            seq INTEGER PRIMARY KEY AUTOINCREMENT,
            id TEXT NOT NULL UNIQUE,
            turn_id TEXT NOT NULL UNIQUE,
            payload TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        INSERT INTO turn_outbox (
            id, turn_id, payload, created_at
        ) VALUES (
            'outbox_1', 'turn_1',
            '{"id":"outbox_1","strand_id":"strand_1","turn_id":"turn_1","external_label":"stim:alice","final_text":"hello","completed_at":"2026-07-22T00:00:00Z"}',
            '2026-07-22T00:00:00Z'
        );
        PRAGMA user_version = 31;
        "#,
    )
    .expect("create v31 schema");
    drop(conn);

    let store = SantiStore::open(&db).expect("migrate v31 to v32");
    assert_eq!(
        santi_core::read_schema_version(&db).expect("schema version"),
        Some(santi_core::SCHEMA_VERSION)
    );
    assert!(
        store
            .list_downstreams()
            .expect("list downstreams")
            .is_empty()
    );
    let batch = store
        .turn_events_since(0, "stim:", 10)
        .expect("read migrated event");
    assert_eq!(batch.events.len(), 1);
    assert_eq!(batch.events[0].external_label, "stim:alice");
    assert_eq!(batch.cursor, 1);

    let conn = Connection::open(db).expect("reopen sqlite");
    let columns: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('downstreams') WHERE name = 'credential_sha256'",
            [],
            |row| row.get(0),
        )
        .expect("digest column");
    assert_eq!(columns, 1);
    let ingest: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'downstream_ingest'",
            [],
            |row| row.get(0),
        )
        .expect("ingest table");
    assert_eq!(ingest, 1);
}
