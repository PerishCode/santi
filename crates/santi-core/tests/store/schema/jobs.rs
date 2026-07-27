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
