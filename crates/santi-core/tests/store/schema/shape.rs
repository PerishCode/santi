use super::*;

#[test]
fn v34() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db = temp.path().join("santi.sqlite");
    let conn = Connection::open(&db).expect("open sqlite");
    conn.execute_batch(
        r#"
        CREATE TABLE strand_effects (
            id TEXT PRIMARY KEY, strand_id TEXT NOT NULL, turn_id TEXT NOT NULL,
            tool_call_id TEXT UNIQUE, effect_type TEXT NOT NULL, state TEXT NOT NULL,
            result_ref TEXT, error_text TEXT, metadata TEXT, created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL, dispatched_at TEXT, settled_at TEXT
        );
        CREATE TABLE thinking_spans (
            id TEXT PRIMARY KEY, turn_id TEXT NOT NULL, provider_response_id TEXT,
            state TEXT NOT NULL, summary TEXT, completion_reason TEXT, error_text TEXT,
            created_at TEXT NOT NULL, updated_at TEXT NOT NULL, finished_at TEXT
        );
        CREATE TABLE inbox_receipts (
            id TEXT PRIMARY KEY, strand_id TEXT NOT NULL, state TEXT NOT NULL,
            accepted_at TEXT NOT NULL, updated_at TEXT NOT NULL
        );
        CREATE TABLE receipt_transitions (
            id TEXT PRIMARY KEY, inbox_id TEXT NOT NULL, sequence INTEGER NOT NULL,
            state TEXT NOT NULL, turn_id TEXT, incident_id TEXT,
            reconstructed_from TEXT, occurred_at TEXT NOT NULL
        );
        CREATE TABLE effect_transitions (
            id TEXT PRIMARY KEY, effect_id TEXT NOT NULL, sequence INTEGER NOT NULL,
            state TEXT NOT NULL, reason TEXT NOT NULL, evidence TEXT,
            occurred_at TEXT NOT NULL
        );
        INSERT INTO strand_effects VALUES (
            'effect_1', 'ss_1', 'turn_1', 'call_1', 'shell', 'confirmed',
            'result_1', NULL, NULL, '2026-07-23T00:00:00Z',
            '2026-07-23T00:00:01Z', '2026-07-23T00:00:00Z',
            '2026-07-23T00:00:01Z'
        );
        INSERT INTO thinking_spans VALUES (
            'thinking_1', 'turn_1', 'response_1', 'completed', NULL,
            'first_text_delta', NULL, '2026-07-23T00:00:00Z',
            '2026-07-23T00:00:01Z', '2026-07-23T00:00:01Z'
        );
        INSERT INTO inbox_receipts VALUES (
            'inbox_1', 'ss_1', 'mechanically_recovered',
            '2026-07-23T00:00:00Z', '2026-07-23T00:00:01Z'
        );
        INSERT INTO receipt_transitions VALUES (
            'transition_1', 'inbox_1', 1, 'turn_failed', 'turn_1', NULL,
            NULL, '2026-07-23T00:00:01Z'
        );
        INSERT INTO effect_transitions VALUES (
            'shift_1', 'effect_1', 1, 'confirmed', 'result_persisted',
            'result_1', '2026-07-23T00:00:01Z'
        );
        PRAGMA user_version = 34;
        "#,
    )
    .expect("seed v34 state");
    drop(conn);

    drop(Store::open(&db).expect("upgrade v34"));
    let conn = Connection::open(&db).expect("reopen sqlite");
    assert_eq!(
        value(&conn, "strand_effects", "state", "effect_1"),
        "settled_applied"
    );
    assert_eq!(
        value(&conn, "thinking_spans", "completion_reason", "thinking_1"),
        "spoke"
    );
    assert_eq!(
        value(&conn, "inbox_receipts", "state", "inbox_1"),
        "recovered"
    );
    assert_eq!(
        value(&conn, "receipt_transitions", "state", "transition_1"),
        "failed"
    );
    let retired: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'effect_transitions'",
            [],
            |row| row.get(0),
        )
        .expect("retired transition table");
    assert_eq!(retired, 0);
}

fn value(conn: &Connection, table: &str, column: &str, id: &str) -> String {
    conn.query_row(
        &format!("SELECT {column} FROM {table} WHERE id = ?1"),
        [id],
        |row| row.get(0),
    )
    .expect("migrated value")
}
