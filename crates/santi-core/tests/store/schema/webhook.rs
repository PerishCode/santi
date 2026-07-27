use super::*;

#[test]
fn v35() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db = temp.path().join("santi.sqlite");
    seed(&db, 35);

    let store = Store::open(&db).expect("upgrade v35");
    let webhooks = store.webhooks().expect("list webhooks");
    assert_eq!(webhooks.len(), 1);
    assert_eq!(webhooks[0].name, "secretary");
    assert_eq!(webhooks[0].soul, "soul_default");
    drop(store);

    assert_eq!(
        santi_core::version(&db).expect("read version"),
        Some(santi_core::VERSION)
    );
    let conn = Connection::open(&db).expect("reopen sqlite");
    let deliveries: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'webhook_deliveries'",
            [],
            |row| row.get(0),
        )
        .expect("delivery table");
    assert_eq!(deliveries, 1);
}

#[test]
fn v33() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db = temp.path().join("santi.sqlite");
    seed(&db, 33);

    let store = Store::open(&db).expect("upgrade v33");
    let webhooks = store.webhooks().expect("list webhooks");
    assert_eq!(webhooks.len(), 1);
    assert_eq!(webhooks[0].name, "secretary");
    assert_eq!(webhooks[0].credential, "SANTI_WEBHOOK_GITHUB_SECRET");
    assert_eq!(
        santi_core::version(&db).expect("read version"),
        Some(santi_core::VERSION)
    );
}

#[test]
fn ensures() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = Store::open(temp.path().join("santi.sqlite")).expect("open store");
    let draft = || santi_core::webhook::Draft {
        name: "secretary".to_string(),
        adaptor: "github".to_string(),
        soul: "soul_default".to_string(),
        strategy: Some("per_thread".to_string()),
        credential: "SANTI_WEBHOOK_GITHUB_SECRET".to_string(),
    };

    let created = store.subscribe(draft()).expect("create");
    let ensured = store.subscribe(draft()).expect("ensure");
    assert_eq!(ensured.created, created.created);
    assert_eq!(store.webhooks().expect("list").len(), 1);

    let error = store
        .subscribe(santi_core::webhook::Draft {
            strategy: Some("single".to_string()),
            ..draft()
        })
        .expect_err("drift must conflict");
    assert!(error.contains("conflicts with an existing subscription"));
    assert_eq!(store.webhooks().expect("list after conflict").len(), 1);
}

fn seed(db: &std::path::Path, version: u32) {
    let conn = Connection::open(db).expect("open sqlite");
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
        INSERT INTO souls (id, created_at, updated_at)
        VALUES ('soul_default', '2026-07-08T00:00:00Z', '2026-07-08T00:00:00Z');
        INSERT INTO webhooks (
            name, adaptor, soul_id, strand_strategy, secret_env, created_at, updated_at
        ) VALUES (
            'secretary', 'github', 'soul_default', 'per_thread',
            'SANTI_WEBHOOK_GITHUB_SECRET', '2026-07-08T00:00:01Z', '2026-07-08T00:00:01Z'
        );
        "#,
    )
    .expect("seed webhook");
    conn.pragma_update(None, "user_version", version)
        .expect("stamp schema");
}
