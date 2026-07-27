use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::Path,
};

use super::model::{Record, Spec};

pub(super) const SPEC: &str = "spec.json";
pub(super) const TERMINAL: &str = "terminal.json";
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

pub(super) fn specify(directory: &Path, spec: &Spec) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(spec).map_err(|error| error.to_string())?;
    let path = directory.join(SPEC);
    if path.exists() {
        let existing = fs::read(&path).map_err(|error| error.to_string())?;
        return if existing == bytes {
            Ok(())
        } else {
            Err("job execution spec conflicts with its retained generation".to_string())
        };
    }
    replace(&path, &bytes)
}

pub(super) fn spec(directory: &Path) -> Result<Spec, String> {
    let bytes = fs::read(directory.join(SPEC)).map_err(|error| error.to_string())?;
    serde_json::from_slice(&bytes).map_err(|error| error.to_string())
}

pub(super) fn terminal(directory: &Path) -> Result<Option<Record>, String> {
    let path = directory.join(TERMINAL);
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| error.to_string())
}

pub(super) fn finish(directory: &Path, terminal: &Record) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(terminal).map_err(|error| error.to_string())?;
    once(&directory.join(TERMINAL), &bytes)
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

fn replace(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temporary = path.with_extension(format!("tmp.{}", std::process::id()));
    let mut file = create(&temporary)?;
    file.write_all(bytes).map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    fs::rename(&temporary, path).map_err(|error| error.to_string())
}

fn once(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if path.exists() {
        return Ok(());
    }
    let temporary = path.with_extension(format!("candidate.{}", std::process::id()));
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
    fs::remove_file(&temporary).map_err(|error| error.to_string())
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
