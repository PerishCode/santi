use super::support::*;

mod more;

#[test]
fn schema_matches_runtime() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db = temp.path().join("santi.sqlite");
    let store = SantiStore::open(&db).expect("open store");
    drop(store);

    let conn = Connection::open(db).expect("open sqlite");
    for table in [
        "souls",
        "messages",
        "message_events",
        "strand_effects",
        "effect_transitions",
        "strands",
        "strand_inbox",
        "inbox_receipts",
        "receipt_transitions",
        "turns",
        "tool_calls",
        "tool_results",
        "thinking_spans",
        "compacts",
        "error_incidents",
        "error_transitions",
        "r_strand_entries",
    ] {
        let exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table],
                |row| row.get(0),
            )
            .expect("table lookup");
        assert_eq!(exists, 1, "missing table {table}");
    }
    for table in [
        "accounts",
        "soul_profiles",
        "soul_sessions",
        "sessions",
        "session_profiles",
        "r_session_messages",
        "session_effects",
        "strand_blocks",
        "rejected_deliveries",
    ] {
        let exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table],
                |row| row.get(0),
            )
            .expect("table lookup");
        assert_eq!(exists, 0, "discarded table {table} still present");
    }
}

#[test]
fn soul_label_anchoring() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = SantiStore::open(temp.path().join("santi.sqlite")).expect("open store");

    let soul = store.create_soul().expect("create soul");
    assert_ne!(soul.id, store.default_soul_id());
    assert!(store.list_souls().expect("list").len() >= 2);
    assert!(store.soul(&soul.id).expect("soul").is_some());

    let s1 = store
        .find_labeled_strand(&soul.id, "github:issue:49")
        .expect("label strand");
    let s1_again = store
        .find_labeled_strand(&soul.id, "github:issue:49")
        .expect("label strand again");
    assert_eq!(s1.id, s1_again.id);
    let s2 = store
        .find_labeled_strand(&soul.id, "github:issue:50")
        .expect("other label");
    assert_ne!(s1.id, s2.id);
    assert_eq!(s1.soul_id, soul.id);
    assert_eq!(store.soul_id_for_strand(&s1.id).expect("soul id"), soul.id);

    let default_strand = store
        .find_labeled_strand(store.default_soul_id(), "github:issue:49")
        .expect("same label, default soul");
    assert_ne!(default_strand.id, s1.id);

    assert!(
        store
            .find_labeled_strand("soul_does_not_exist", "github:issue:99")
            .is_err()
    );
}

#[test]
fn absent_schema_is_none() {
    let temp = tempfile::tempdir().expect("temp dir");
    let missing = temp.path().join("nope.sqlite");
    assert_eq!(
        santi_core::read_schema_version(&missing).expect("read"),
        None
    );
}

#[test]
fn schema_migration_preserves_webhooks() {
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
                created_at TEXT NOT NULL
            );
            INSERT INTO souls (id, created_at, updated_at)
            VALUES ('soul_default', '2026-07-08T00:00:00Z', '2026-07-08T00:00:00Z');
            INSERT INTO webhooks (
                name, adaptor, soul_id, strand_strategy, secret_env, created_at, updated_at
            ) VALUES (
                'secretary', 'github', 'soul_default', 'per_thread',
                'SANTI_WEBHOOK_GITHUB_SECRET', '2026-07-08T00:00:01Z', '2026-07-08T00:00:01Z'
            );
            INSERT INTO strand_inbox (id, strand_id, message_kind, content, created_at)
            VALUES ('inbox_old', 'ss_existing', 'text', '{"parts":[]}', '2026-07-08T00:00:02Z');
            PRAGMA user_version = 21;
            "#,
        )
        .expect("seed v21 db");
    }

    let store = SantiStore::open(&db).expect("open migrates v21 to current schema");
    assert_eq!(
        santi_core::read_schema_version(&db).expect("read version"),
        Some(santi_core::SCHEMA_VERSION)
    );
    let webhooks = store.list_webhooks().expect("list webhooks");
    assert_eq!(webhooks.len(), 1);
    let webhook = &webhooks[0];
    assert_eq!(webhook.name, "secretary");
    assert_eq!(webhook.adaptor, "github");
    assert_eq!(webhook.soul_id, "soul_default");
    assert_eq!(webhook.strand_strategy, "per_thread");
    assert_eq!(webhook.secret_env, "SANTI_WEBHOOK_GITHUB_SECRET");
    assert!(store.soul("soul_default").expect("soul").is_some());
    drop(store);

    let conn = Connection::open(&db).expect("open sqlite");
    for column in ["source_type", "source_ref", "source_metadata"] {
        let exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('strand_inbox') WHERE name = ?1",
                [column],
                |row| row.get(0),
            )
            .expect("column lookup");
        assert_eq!(exists, 1, "missing migrated column {column}");
    }
    for column in ["created_at", "metadata"] {
        let exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('compacts') WHERE name = ?1",
                [column],
                |row| row.get(0),
            )
            .expect("compact column lookup");
        assert_eq!(exists, 1, "missing migrated compact column {column}");
    }
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
    let pending_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM strand_inbox WHERE id = 'inbox_old'",
            [],
            |row| row.get(0),
        )
        .expect("pending row count");
    assert_eq!(pending_count, 1, "v21 pending inbox row was wiped");
    let webhook_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM webhooks WHERE name = 'secretary'",
            [],
            |row| row.get(0),
        )
        .expect("webhook count");
    assert_eq!(webhook_count, 1, "webhook subscription was wiped");
}

#[test]
fn schema_read_matches_open() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db = temp.path().join("santi.sqlite");

    {
        let conn = Connection::open(&db).expect("open sqlite");
        conn.pragma_update(None, "user_version", 5u32)
            .expect("stamp version");
    }
    assert_eq!(
        santi_core::read_schema_version(&db).expect("read"),
        Some(5),
        "probe must report the stored version, not migrate it"
    );
    assert_eq!(
        santi_core::read_schema_version(&db).expect("read again"),
        Some(5),
        "a second probe still sees the stale version — the first was read-only"
    );

    let store = SantiStore::open(&db).expect("open store");
    drop(store);
    assert_eq!(
        santi_core::read_schema_version(&db).expect("read post-open"),
        Some(santi_core::SCHEMA_VERSION)
    );
}

#[test]
fn memory_path_composes() {
    let path = santi_core::soul_memory_file("/srv/santi/runtime", "soul_default");
    assert!(path.ends_with("souls/soul_default/memory/MEMORY.md"));
    assert!(path.starts_with("/srv/santi/runtime"));
}
