use santi_estate::Store;
use sqlx::sqlite::{SqliteConnectOptions, SqliteConnection};
use sqlx::{AssertSqlSafe, Connection, Executor};
use std::path::{Path, PathBuf};

const VERSION: i64 = 39;
const LEGACY: &str = include_str!("legacy-v39.sql");
const RETIRED: &str = r#"
CREATE TABLE im_inbox (id INTEGER);
CREATE TABLE im_participants (id INTEGER);
CREATE TABLE r_soul_session_messages (id INTEGER);
CREATE INDEX idx_im_inbox_participant_seq ON im_inbox(id);
CREATE INDEX idx_im_inbox_turn ON im_inbox(id);
CREATE INDEX idx_r_soul_session_messages_seq ON r_soul_session_messages(id);
CREATE INDEX idx_r_soul_session_messages_target_lookup ON r_soul_session_messages(id);
"#;

#[tokio::test]
async fn quarantines_exact_legacy_store() {
    let temp = tempfile::tempdir().expect("temp");
    let path = temp.path().join("estate.sqlite");
    fixture(&path, VERSION).await;

    let store = Store::open(&path).await.expect("transition");
    assert!(store.souls().await.expect("souls").is_empty());
    drop(store);

    let dirs = quarantines(&path);
    assert_eq!(dirs.len(), 1);
    let manifest = manifest(&dirs[0]);
    assert_eq!(manifest["state"], "ready");
    assert_eq!(manifest["legacy_version"], VERSION);
    assert!(dirs[0].join("estate.sqlite").exists());
    Store::open(&path).await.expect("reopen estate");
}

#[tokio::test]
async fn quarantines_retired_im_shape() {
    let temp = tempfile::tempdir().expect("temp");
    let path = temp.path().join("estate.sqlite");
    fixture(&path, VERSION).await;
    execute(&path, RETIRED).await;

    let store = Store::open(&path).await.expect("transition");
    assert!(store.souls().await.expect("souls").is_empty());
    drop(store);

    let dirs = quarantines(&path);
    assert_eq!(dirs.len(), 1);
    assert_eq!(manifest(&dirs[0])["state"], "ready");
    assert!(dirs[0].join("estate.sqlite").exists());
}

#[tokio::test]
async fn refuses_unknown_legacy_shapes() {
    let temp = tempfile::tempdir().expect("temp");
    let path = temp.path().join("drift.sqlite");
    fixture(&path, VERSION).await;
    execute(&path, "CREATE TABLE unknown_resource (id INTEGER)").await;
    let error = match Store::open(&path).await {
        Ok(_) => panic!("drift must refuse"),
        Err(error) => error,
    };
    assert!(error.contains("shape is not exact"));
    assert!(path.exists());

    let path = temp.path().join("old.sqlite");
    fixture(&path, VERSION - 1).await;
    let error = match Store::open(&path).await {
        Ok(_) => panic!("old version must refuse"),
        Err(error) => error,
    };
    assert!(error.contains("unsupported legacy database version 38"));
    assert!(path.exists());
}

#[tokio::test]
async fn resumes_interrupted_moves() {
    let temp = tempfile::tempdir().expect("temp");
    let path = temp.path().join("estate.sqlite");
    fixture(&path, VERSION).await;
    let root = root(&path);
    let moving = root.join(".moving-legacy-v39-test");
    std::fs::create_dir_all(&moving).expect("moving");
    let source = std::fs::canonicalize(temp.path())
        .expect("parent")
        .join("estate.sqlite");
    let held = serde_json::json!({
        "schema": "santi.legacy-quarantine.v1",
        "state": "moving",
        "legacy_version": VERSION,
        "source": source.display().to_string(),
        "created": "2026-07-28T00:00:00.000Z",
        "files": ["estate.sqlite"],
    });
    std::fs::write(
        moving.join("transition.json"),
        serde_json::to_vec_pretty(&held).expect("json"),
    )
    .expect("manifest");
    std::fs::rename(&path, moving.join("estate.sqlite")).expect("partial move");

    Store::open(&path).await.expect("resume");
    let ready = root.join("legacy-v39-test");
    assert_eq!(manifest(&ready)["state"], "ready");
    assert!(ready.join("estate.sqlite").exists());
    assert!(path.exists());
}

#[tokio::test]
async fn refuses_orphan_sidecars() {
    let temp = tempfile::tempdir().expect("temp");
    let path = temp.path().join("estate.sqlite");
    std::fs::write(path.with_file_name("estate.sqlite-wal"), b"orphan").expect("sidecar");
    let error = match Store::open(&path).await {
        Ok(_) => panic!("orphan must refuse"),
        Err(error) => error,
    };
    assert!(error.contains("orphan SQLite sidecar"));
}

async fn fixture(path: &Path, version: i64) {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true);
    let mut conn = SqliteConnection::connect_with(&options)
        .await
        .expect("fixture db");
    conn.execute(sqlx::raw_sql(AssertSqlSafe(LEGACY.to_string())))
        .await
        .expect("legacy schema");
    conn.execute(sqlx::raw_sql(AssertSqlSafe(format!(
        "PRAGMA user_version = {version}"
    ))))
    .await
    .expect("version");
    conn.close().await.expect("close");
}

async fn execute(path: &Path, sql: &str) {
    let options = SqliteConnectOptions::new().filename(path);
    let mut conn = SqliteConnection::connect_with(&options)
        .await
        .expect("open fixture");
    conn.execute(sqlx::raw_sql(AssertSqlSafe(sql.to_string())))
        .await
        .expect("execute");
    conn.close().await.expect("close");
}

fn quarantines(path: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(root(path))
        .expect("quarantine")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("legacy-v39-"))
        })
        .collect()
}

fn manifest(dir: &Path) -> serde_json::Value {
    serde_json::from_slice(
        &std::fs::read(dir.join("transition.json")).expect("read transition manifest"),
    )
    .expect("transition manifest")
}

fn root(path: &Path) -> PathBuf {
    path.with_file_name(format!(
        "{}.quarantine",
        path.file_name().expect("filename").to_string_lossy()
    ))
}
