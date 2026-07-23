use super::support::*;

mod downstream;
mod more;
mod retire;

#[test]
fn schema_matches_runtime() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db = temp.path().join("santi.sqlite");
    let store = Store::open(&db).expect("open store");
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
        "reply_outbox",
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
    let store = Store::open(temp.path().join("santi.sqlite")).expect("open store");

    let soul = store.awaken().expect("create soul");
    assert_ne!(soul.id, store.genesis());
    assert!(store.souls().expect("list").len() >= 2);
    assert!(store.soul(&soul.id).expect("soul").is_some());

    let s1 = store
        .labeled(&soul.id, "github:issue:49")
        .expect("label strand");
    let s1_again = store
        .labeled(&soul.id, "github:issue:49")
        .expect("label strand again");
    assert_eq!(s1.id, s1_again.id);
    let s2 = store
        .labeled(&soul.id, "github:issue:50")
        .expect("other label");
    assert_ne!(s1.id, s2.id);
    assert_eq!(s1.soul, soul.id);
    assert_eq!(store.keeper(&s1.id).expect("soul id"), soul.id);

    let default_strand = store
        .labeled(store.genesis(), "github:issue:49")
        .expect("same label, default soul");
    assert_ne!(default_strand.id, s1.id);

    assert!(
        store
            .labeled("soul_does_not_exist", "github:issue:99")
            .is_err()
    );
}

#[test]
fn absent_schema_is_none() {
    let temp = tempfile::tempdir().expect("temp dir");
    let missing = temp.path().join("nope.sqlite");
    assert_eq!(santi_core::version(&missing).expect("read"), None);
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
        santi_core::version(&db).expect("read"),
        Some(5),
        "probe must report the stored version, not migrate it"
    );
    assert_eq!(
        santi_core::version(&db).expect("read again"),
        Some(5),
        "a second probe still sees the stale version — the first was read-only"
    );

    let store = Store::open(&db).expect("open store");
    drop(store);
    assert_eq!(
        santi_core::version(&db).expect("read post-open"),
        Some(santi_core::VERSION)
    );
}

#[test]
fn memory_path_composes() {
    let path = santi_core::memoir("/srv/santi/runtime", "soul_default");
    assert!(path.ends_with("souls/soul_default/memory/MEMORY.md"));
    assert!(path.starts_with("/srv/santi/runtime"));
}
