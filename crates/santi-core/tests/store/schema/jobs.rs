use super::*;

#[test]
fn upgrades() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db = temp.path().join("santi.sqlite");
    let conn = Connection::open(&db).expect("open sqlite");
    conn.execute_batch(
        r#"
        CREATE TABLE souls (
            id TEXT PRIMARY KEY,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        INSERT INTO souls (id, created_at, updated_at)
        VALUES ('soul_existing', '2026-07-27T00:00:00.000Z', '2026-07-27T00:00:00.000Z');
        PRAGMA user_version = 36;
        "#,
    )
    .expect("seed v36");
    drop(conn);

    let store = Store::open(&db).expect("upgrade v36");
    assert!(store.soul("soul_existing").expect("query soul").is_some());
    drop(store);
    assert_eq!(
        santi_core::version(&db).expect("schema version"),
        Some(santi_core::VERSION)
    );
    let conn = Connection::open(&db).expect("reopen sqlite");
    for table in ["jobs", "job_capabilities"] {
        let exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table],
                |row| row.get(0),
            )
            .expect("table lookup");
        assert_eq!(exists, 1, "missing table {table}");
    }
}

#[test]
fn expands() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db = temp.path().join("santi.sqlite");
    let conn = Connection::open(&db).expect("open sqlite");
    conn.execute_batch(
        r#"
        CREATE TABLE jobs (
            id TEXT PRIMARY KEY,
            soul_id TEXT NOT NULL,
            strand_id TEXT NOT NULL,
            turn_id TEXT NOT NULL,
            tool_call_id TEXT NOT NULL,
            effect_id TEXT NOT NULL,
            description TEXT NOT NULL,
            command TEXT NOT NULL,
            cwd TEXT,
            timeout_seconds INTEGER NOT NULL,
            output_limit_bytes INTEGER NOT NULL,
            request_sha256 TEXT NOT NULL,
            capability_sha256 TEXT NOT NULL UNIQUE,
            generation TEXT NOT NULL UNIQUE,
            supervisor_ref TEXT NOT NULL UNIQUE,
            state TEXT NOT NULL,
            reason TEXT,
            exit_code INTEGER,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            accepted_at TEXT,
            started_at TEXT,
            finished_at TEXT,
            acknowledged_at TEXT
        );
        CREATE TABLE strand_inbox (
            id TEXT PRIMARY KEY,
            strand_id TEXT NOT NULL,
            message_kind TEXT NOT NULL,
            content TEXT NOT NULL,
            source_type TEXT,
            source_ref TEXT,
            source_metadata TEXT,
            created_at TEXT NOT NULL
        );
        PRAGMA user_version = 37;
        "#,
    )
    .expect("seed v37");
    drop(conn);

    let store = Store::open(&db).expect("upgrade v37");
    drop(store);
    let conn = Connection::open(&db).expect("reopen sqlite");
    for (table, column) in [
        ("jobs", "remind_every_seconds"),
        ("jobs", "attention_revision"),
        ("jobs", "reminder_tick"),
        ("strand_inbox", "coalesce_key"),
        ("strand_inbox", "coalesce_causes"),
    ] {
        let mut stmt = conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .expect("prepare columns");
        let columns = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .expect("query columns")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect columns");
        assert!(
            columns.iter().any(|held| held == column),
            "{table}.{column}"
        );
    }
    let slots: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'inbox_slots'",
            [],
            |row| row.get(0),
        )
        .expect("slot table");
    assert_eq!(slots, 1);
}
