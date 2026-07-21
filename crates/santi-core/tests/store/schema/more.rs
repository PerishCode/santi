use super::*;

#[test]
fn schema_migrates_live_shape() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db = temp.path().join("santi.sqlite");
    {
        let conn = Connection::open(&db).expect("open sqlite");
        conn.execute_batch(
            r#"
            CREATE TABLE souls (
                id TEXT PRIMARY KEY,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE webhooks (
                name TEXT PRIMARY KEY,
                adaptor TEXT NOT NULL,
                soul_id TEXT NOT NULL,
                strand_strategy TEXT NOT NULL,
                secret_env TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE strand_inbox (
                id TEXT PRIMARY KEY,
                strand_id TEXT NOT NULL,
                message_kind TEXT NOT NULL CHECK (message_kind IN ('text', 'santi_system')),
                content TEXT NOT NULL,
                source_type TEXT,
                source_ref TEXT,
                source_metadata TEXT,
                created_at TEXT NOT NULL
            );
            CREATE TABLE compacts (
                id TEXT PRIMARY KEY,
                strand_id TEXT NOT NULL,
                summary TEXT NOT NULL,
                start_message_id TEXT NOT NULL,
                end_message_id TEXT NOT NULL
            );
            INSERT INTO souls (id, created_at, updated_at)
            VALUES ('soul_default', '2026-07-08T00:00:00Z', '2026-07-08T00:00:00Z');
            INSERT INTO webhooks (
                name, adaptor, soul_id, strand_strategy, secret_env, created_at, updated_at
            ) VALUES (
                'secretary', 'github', 'soul_default', 'per_thread',
                'SANTI_WEBHOOK_GITHUB_SECRET', '2026-07-08T00:00:01Z', '2026-07-08T00:00:01Z'
            );
            INSERT INTO strand_inbox (
                id, strand_id, message_kind, content, source_type, source_ref, source_metadata, created_at
            ) VALUES (
                'inbox_v22', 'ss_existing', 'text', '{"parts":[]}',
                'webhook', 'github:secretary:issue:1', '{"delivery":"abc"}',
                '2026-07-08T00:00:02Z'
            );
            INSERT INTO compacts (id, strand_id, summary, start_message_id, end_message_id)
            VALUES ('cmp_v22', 'ss_existing', 'old compact', 'msg_a', 'msg_b');
            PRAGMA user_version = 22;
            "#,
        )
        .expect("seed v22 db");
    }

    let store = SantiStore::open(&db).expect("open migrates v22 to current schema");
    assert_eq!(
        santi_core::read_schema_version(&db).expect("read version"),
        Some(santi_core::SCHEMA_VERSION)
    );
    let webhooks = store.list_webhooks().expect("list webhooks");
    assert_eq!(webhooks.len(), 1);
    assert_eq!(webhooks[0].name, "secretary");
    drop(store);

    let conn = Connection::open(&db).expect("open sqlite");
    let pending: (String, String, String) = conn
        .query_row(
            r#"
            SELECT source_type, source_ref, source_metadata
            FROM strand_inbox
            WHERE id = 'inbox_v22'
            "#,
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("pending inbox row");
    assert_eq!(pending.0, "webhook");
    assert_eq!(pending.1, "github:secretary:issue:1");
    assert!(pending.2.contains("delivery"));
    let compact: (String, Option<String>, Option<String>) = conn
        .query_row(
            r#"
            SELECT summary, created_at, metadata
            FROM compacts
            WHERE id = 'cmp_v22'
            "#,
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("compact row");
    assert_eq!(compact.0, "old compact");
    assert!(compact.1.is_none());
    assert!(compact.2.is_none());
    for table in ["error_incidents", "error_transitions"] {
        let exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table],
                |row| row.get(0),
            )
            .expect("table lookup");
        assert_eq!(exists, 1, "missing migrated table {table}");
    }
    for table in ["strand_blocks", "rejected_deliveries"] {
        let exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table],
                |row| row.get(0),
            )
            .expect("table lookup");
        assert_eq!(exists, 0, "replaced table {table} still present");
    }
}

#[test]
fn v23_aggregates_incident() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db = temp.path().join("santi.sqlite");
    {
        let conn = Connection::open(&db).expect("open sqlite");
        conn.execute_batch(
            r#"
            CREATE TABLE strand_blocks (
                id TEXT PRIMARY KEY,
                strand_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                status TEXT NOT NULL,
                reason_code TEXT NOT NULL,
                reason_text TEXT NOT NULL,
                provider TEXT,
                model TEXT,
                budget_source TEXT,
                budget_bytes INTEGER,
                input_items INTEGER,
                input_bytes INTEGER,
                instructions_bytes INTEGER,
                tools_bytes INTEGER,
                total_bytes INTEGER,
                observed_turn_id TEXT,
                observed_at_seq INTEGER,
                metadata TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                cleared_at TEXT,
                cleared_by TEXT
            );
            CREATE TABLE rejected_deliveries (
                id TEXT PRIMARY KEY,
                strand_id TEXT,
                block_id TEXT,
                source_type TEXT,
                source_ref TEXT,
                source_metadata TEXT,
                message_kind TEXT,
                content_sha256 TEXT NOT NULL,
                content_bytes INTEGER NOT NULL,
                content_excerpt TEXT NOT NULL,
                reason_code TEXT NOT NULL,
                reason_text TEXT NOT NULL,
                received_at TEXT NOT NULL
            );
            INSERT INTO strand_blocks (
                id, strand_id, kind, status, reason_code, reason_text, provider,
                model, budget_source, budget_bytes, input_items, input_bytes,
                instructions_bytes, tools_bytes, total_bytes, observed_turn_id,
                observed_at_seq, metadata, created_at, updated_at, cleared_at, cleared_by
            ) VALUES (
                'blk_abc', 'ss_1', 'context_over_budget', 'active',
                'candidate_input_exceeds_budget', 'over budget', 'openai', 'gpt',
                'config', 100, 2, 120, 10, 5, 135, NULL, 4,
                '{"phase":"ingest_admission"}', 't1', 't2', NULL, NULL
            );
            INSERT INTO rejected_deliveries (
                id, strand_id, block_id, content_sha256, content_bytes,
                content_excerpt, reason_code, reason_text, received_at
            ) VALUES
                ('reject_1', 'ss_1', 'blk_abc', 'a', 10, 'first', 'x', 'over budget', 't1'),
                ('reject_2', 'ss_1', 'blk_abc', 'b', 20, 'second', 'x', 'over budget', 't2');
            PRAGMA user_version = 23;
            "#,
        )
        .expect("seed v23 db");
    }

    let store = SantiStore::open(&db).expect("open migrates v23 to v24");
    drop(store);
    let conn = Connection::open(&db).expect("open sqlite");
    let incident: (String, String, i64, i64) = conn
        .query_row(
            "SELECT id, status, occurrence_count, revision FROM error_incidents",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("migrated incident");
    assert_eq!(
        incident,
        ("inc_abc".to_string(), "active".to_string(), 2, 1)
    );
    let transitions: i64 = conn
        .query_row("SELECT COUNT(*) FROM error_transitions", [], |row| {
            row.get(0)
        })
        .expect("transition count");
    assert_eq!(
        transitions, 0,
        "migration must not replay historical lifecycle"
    );
    for table in ["strand_blocks", "rejected_deliveries"] {
        let exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table],
                |row| row.get(0),
            )
            .expect("table lookup");
        assert_eq!(exists, 0, "replaced table {table} still present");
    }
}

#[test]
fn v27_window_migrates_preserving_state() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db = temp.path().join("db");
    {
        let store = SantiStore::open(&db).expect("create v28 store");
        store.create_soul().expect("soul");
        let soul = store.list_souls().expect("souls").remove(0);
        store
            .create_webhook(santi_core::CreateWebhookRequest {
                name: "secretary".to_string(),
                adaptor: "feishu".to_string(),
                soul_id: soul.id,
                strand_strategy: None,
                secret_env: "SECRETARY_SECRET".to_string(),
            })
            .expect("webhook");
        drop(store);
    }
    {
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch(
            r#"
            DROP TABLE window_messages;
            PRAGMA user_version = 27;
            "#,
        )
        .unwrap();
    }
    let store = SantiStore::open(&db).expect("migrate v27 to v28");
    assert_eq!(
        santi_core::read_schema_version(&db)
            .expect("version")
            .expect("some"),
        santi_core::SCHEMA_VERSION
    );
    let webhooks = store.list_webhooks().expect("webhooks");
    assert!(
        webhooks.iter().any(|hook| hook.name == "secretary"),
        "webhooks survive the v27 upgrade"
    );
    let souls = store.list_souls().expect("souls");
    assert!(!souls.is_empty(), "souls survive the v27 upgrade");
    let conn = rusqlite::Connection::open(&db).unwrap();
    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM window_messages", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 0, "window_messages exists and starts empty");
}
