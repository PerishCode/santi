use fs2::FileExt;
use sqlx::sqlite::{SqliteConnectOptions, SqliteConnection};
use sqlx::{Connection, Row};
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

mod manifest;
mod signature;

use manifest::{Manifest, State};

pub(in crate::store) async fn prepare(path: &Path) -> Result<Option<PathBuf>, String> {
    let _lock = lock(path)?;
    if let Some((dir, held)) = pending(path)? {
        return move_files(path, &dir, held).map(Some);
    }
    if !path.exists() {
        refuse_orphans(path)?;
        return Ok(None);
    }
    refuse_non_file(path)?;
    let (version, objects) = probe(path).await?;
    if version == 0 {
        return Ok(None);
    }
    if version != signature::VERSION {
        return Err(format!(
            "unsupported legacy database version {version}; expected {}; database left unchanged",
            signature::VERSION
        ));
    }
    if !signature::exact(&objects) {
        return Err("legacy database v39 shape is not exact; database left unchanged".to_string());
    }
    exclusive(path).await?;
    let dir = generation(path)?;
    let manifest = Manifest::moving(source(path)?, files(path)?);
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    manifest.write(&dir)?;
    move_files(path, &dir, manifest).map(Some)
}

fn lock(path: &Path) -> Result<File, String> {
    let lock = sibling(path, ".transition.lock")?;
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .truncate(false)
        .write(true)
        .open(&lock)
        .map_err(|error| format!("open transition lock {}: {error}", lock.display()))?;
    FileExt::try_lock_exclusive(&file).map_err(|error| {
        format!(
            "another database transition holds {}: {error}",
            lock.display()
        )
    })?;
    Ok(file)
}

async fn probe(path: &Path) -> Result<(i64, Vec<String>), String> {
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

async fn exclusive(path: &Path) -> Result<(), String> {
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

fn pending(path: &Path) -> Result<Option<(PathBuf, Manifest)>, String> {
    let root = root(path)?;
    if !root.exists() {
        return Ok(None);
    }
    let source = source(path)?;
    let mut held = Vec::new();
    for entry in std::fs::read_dir(&root).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        if !entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_dir()
        {
            return Err(format!(
                "unknown legacy quarantine artifact {}",
                entry.path().display()
            ));
        }
        let name = entry.file_name();
        let name = name
            .to_str()
            .ok_or_else(|| "non-UTF-8 legacy quarantine generation".to_string())?;
        if name.starts_with("legacy-v39-") {
            continue;
        }
        if !name.starts_with(".moving-legacy-v39-") {
            return Err(format!(
                "unknown legacy quarantine generation {}",
                entry.path().display()
            ));
        }
        let manifest = Manifest::read(&entry.path())?;
        if manifest.source == source {
            held.push((entry.path(), manifest));
        }
    }
    match held.len() {
        0 => Ok(None),
        1 => Ok(held.pop()),
        _ => Err("multiple pending legacy database transitions".to_string()),
    }
}

fn move_files(path: &Path, dir: &Path, mut manifest: Manifest) -> Result<PathBuf, String> {
    if manifest.state == State::Moving {
        for name in &manifest.files {
            let from = path.parent().unwrap_or_else(|| Path::new(".")).join(name);
            let to = dir.join(name);
            move_one(&from, &to, name)?;
        }
        manifest.state = State::Ready;
        manifest.write(dir)?;
    }
    let final_dir = ready(dir)?;
    if final_dir.exists() {
        return Err(format!(
            "legacy quarantine generation already exists: {}",
            final_dir.display()
        ));
    }
    std::fs::rename(dir, &final_dir).map_err(|error| error.to_string())?;
    sync(final_dir.parent().unwrap_or_else(|| Path::new(".")))?;
    Ok(final_dir)
}

fn move_one(from: &Path, to: &Path, name: &str) -> Result<(), String> {
    match (from.exists(), to.exists()) {
        (true, false) => {
            refuse_non_file(from)?;
            std::fs::rename(from, to).map_err(|error| {
                format!("quarantine {} as {}: {error}", from.display(), to.display())
            })
        }
        (false, true) => Ok(()),
        (true, true) => Err(format!(
            "legacy transition has both source and quarantine file {name}"
        )),
        (false, false) => Err(format!("legacy transition lost file {name}")),
    }
}

fn files(path: &Path) -> Result<Vec<String>, String> {
    let name = filename(path)?;
    let mut files = vec![name.clone()];
    for suffix in ["-wal", "-shm", "-journal"] {
        let candidate = path.with_file_name(format!("{name}{suffix}"));
        if candidate.exists() {
            refuse_non_file(&candidate)?;
            files.push(format!("{name}{suffix}"));
        }
    }
    Ok(files)
}

fn refuse_orphans(path: &Path) -> Result<(), String> {
    let name = filename(path)?;
    for suffix in ["-wal", "-shm", "-journal"] {
        let candidate = path.with_file_name(format!("{name}{suffix}"));
        if candidate.exists() {
            return Err(format!(
                "orphan SQLite sidecar blocks fresh estate: {}",
                candidate.display()
            ));
        }
    }
    Ok(())
}

fn refuse_non_file(path: &Path) -> Result<(), String> {
    let kind = std::fs::symlink_metadata(path)
        .map_err(|error| error.to_string())?
        .file_type();
    if kind.is_file() && !kind.is_symlink() {
        Ok(())
    } else {
        Err(format!(
            "database transition target is not a file: {}",
            path.display()
        ))
    }
}

fn generation(path: &Path) -> Result<PathBuf, String> {
    Ok(root(path)?.join(format!(
        ".moving-legacy-v{}-{}",
        signature::VERSION,
        santi_model::tag("q")
    )))
}

fn ready(path: &Path) -> Result<PathBuf, String> {
    let name = filename(path)?;
    let name = name
        .strip_prefix(".moving-")
        .ok_or_else(|| format!("invalid moving quarantine generation {name}"))?;
    Ok(path.with_file_name(name))
}

fn root(path: &Path) -> Result<PathBuf, String> {
    let name = filename(path)?;
    Ok(path.with_file_name(format!("{name}.quarantine")))
}

fn sibling(path: &Path, suffix: &str) -> Result<PathBuf, String> {
    let name = filename(path)?;
    Ok(path.with_file_name(format!("{name}{suffix}")))
}

fn source(path: &Path) -> Result<String, String> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let parent = std::fs::canonicalize(parent).map_err(|error| error.to_string())?;
    Ok(parent.join(filename(path)?).display().to_string())
}

fn filename(path: &Path) -> Result<String, String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .ok_or_else(|| format!("database path has no UTF-8 filename: {}", path.display()))
}

#[cfg(unix)]
fn sync(path: &Path) -> Result<(), String> {
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|error| error.to_string())
}

#[cfg(not(unix))]
fn sync(_path: &Path) -> Result<(), String> {
    Ok(())
}
