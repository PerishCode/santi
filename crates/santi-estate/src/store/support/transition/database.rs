use sqlx::sqlite::{SqliteConnectOptions, SqliteConnection};
use sqlx::{Connection, Row};
use std::path::Path;

pub(super) async fn probe(path: &Path) -> Result<(i64, Vec<String>), String> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .read_only(true)
        .create_if_missing(false)
        .busy_timeout(std::time::Duration::ZERO);
    let mut conn = SqliteConnection::connect_with(&options)
        .await
        .map_err(|error| format!("open database {}: {error}", path.display()))?;
    let version = sqlx::query_scalar::<_, i64>("PRAGMA user_version")
        .fetch_one(&mut conn)
        .await
        .map_err(|error| error.to_string())?;
    let rows = sqlx::query(
        "SELECT type, name, tbl_name FROM sqlite_master \
         WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name, tbl_name",
    )
    .fetch_all(&mut conn)
    .await
    .map_err(|error| error.to_string())?;
    let mut objects = Vec::with_capacity(rows.len());
    for row in rows {
        objects.push(format!(
            "{}|{}|{}",
            row.try_get::<String, _>(0)
                .map_err(|error| error.to_string())?,
            row.try_get::<String, _>(1)
                .map_err(|error| error.to_string())?,
            row.try_get::<String, _>(2)
                .map_err(|error| error.to_string())?
        ));
    }
    conn.close().await.map_err(|error| error.to_string())?;
    Ok((version, objects))
}

pub(super) async fn exclusive(path: &Path) -> Result<(), String> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(false)
        .busy_timeout(std::time::Duration::ZERO);
    let mut conn = SqliteConnection::connect_with(&options)
        .await
        .map_err(|error| format!("open legacy database {}: {error}", path.display()))?;
    sqlx::query("BEGIN EXCLUSIVE")
        .execute(&mut conn)
        .await
        .map_err(|error| format!("legacy database must be closed before transition: {error}"))?;
    sqlx::query("ROLLBACK")
        .execute(&mut conn)
        .await
        .map_err(|error| error.to_string())?;
    conn.close().await.map_err(|error| error.to_string())
}
