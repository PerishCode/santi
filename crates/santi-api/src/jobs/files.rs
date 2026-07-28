use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::Path,
};

use super::model::{Phase, Record, Snapshot, Spec};

pub(super) const SPEC: &str = "spec.json";
pub(super) const STATE: &str = "state.json";
pub(super) const RESULT: &str = "result.json";
const TERMINAL: &str = "terminal.json";
pub(super) const CANCEL: &str = "cancel.requested";
pub(super) const LIMIT: &str = "output_limit.reached";

pub(super) fn prepare(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub(super) fn specify(directory: &Path, requested: &Spec) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(requested).map_err(|error| error.to_string())?;
    let path = directory.join(SPEC);
    if path.exists() {
        let existing = spec(directory)?;
        return if existing.matches(requested) {
            Ok(())
        } else {
            Err("job execution spec conflicts with its retained stamp".to_string())
        };
    }
    replace(&path, &bytes)
}

pub(super) fn spec(directory: &Path) -> Result<Spec, String> {
    let bytes = fs::read(directory.join(SPEC)).map_err(|error| error.to_string())?;
    serde_json::from_slice(&bytes).map_err(|error| error.to_string())
}

pub(super) fn state(directory: &Path) -> Result<Option<Snapshot>, String> {
    let path = directory.join(STATE);
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let state: Snapshot = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    if state.schema == "santi.job.state.v1" {
        Ok(Some(state))
    } else {
        Err(format!("unsupported job state schema {}", state.schema))
    }
}

pub(super) fn advance(directory: &Path, phase: Phase) -> Result<(), String> {
    if let Some(current) = state(directory)? {
        let valid = matches!(
            (current.phase, phase),
            (
                Phase::Claimed,
                Phase::Claimed | Phase::Running | Phase::Terminal
            ) | (Phase::Running, Phase::Running | Phase::Terminal)
                | (Phase::Terminal, Phase::Terminal)
        );
        if !valid {
            return Err("job state cannot move backwards".to_string());
        }
        if current.phase == phase {
            return Ok(());
        }
    } else if phase != Phase::Claimed {
        return Err("job state must begin at claimed".to_string());
    }
    let bytes =
        serde_json::to_vec_pretty(&Snapshot::new(phase)).map_err(|error| error.to_string())?;
    replace(&directory.join(STATE), &bytes)
}

pub(super) fn terminal(directory: &Path) -> Result<Option<Record>, String> {
    for name in [RESULT, TERMINAL] {
        match fs::read(directory.join(name)) {
            Ok(bytes) => {
                return serde_json::from_slice(&bytes)
                    .map(Some)
                    .map_err(|error| error.to_string());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(None)
}

pub(super) fn finish(directory: &Path, terminal: &Record) -> Result<(), String> {
    if self::terminal(directory)?.is_some() {
        return Ok(());
    }
    let bytes = serde_json::to_vec_pretty(terminal).map_err(|error| error.to_string())?;
    once(&directory.join(RESULT), &bytes)
}

pub(super) fn mark(directory: &Path, name: &str) -> Result<(), String> {
    once(&directory.join(name), b"1\n")
}

pub(super) fn log(directory: &Path, name: &str) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(directory.join(name))
        .map_err(|error| error.to_string())
}

#[cfg(target_os = "macos")]
pub(super) fn artifact(path: &Path, name: &str, bytes: &[u8]) -> Result<(), String> {
    replace(&path.join(name), bytes)
}

fn replace(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temporary = path.with_extension(format!("tmp.{}", uuid::Uuid::new_v4().simple()));
    let mut file = create(&temporary)?;
    file.write_all(bytes).map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    fs::rename(&temporary, path).map_err(|error| error.to_string())?;
    sync(path)
}

fn once(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if path.exists() {
        return Ok(());
    }
    let temporary = path.with_extension(format!("candidate.{}", uuid::Uuid::new_v4().simple()));
    let mut file = create(&temporary)?;
    file.write_all(bytes).map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    match fs::hard_link(&temporary, path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            return Err(error.to_string());
        }
    }
    fs::remove_file(&temporary).map_err(|error| error.to_string())?;
    sync(path)
}

fn create(path: &Path) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).map_err(|error| error.to_string())
}

fn sync(path: &Path) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "job artifact has no parent directory".to_string())?;
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| error.to_string())
}
