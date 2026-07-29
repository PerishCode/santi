use fs2::FileExt;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use super::manifest::{Manifest, State};
use super::signature;

pub(super) struct Estate<'a>(&'a Path);

impl<'a> Estate<'a> {
    pub fn new(path: &'a Path) -> Self {
        Self(path)
    }

    pub fn lock(&self) -> Result<File, String> {
        let lock = self.sibling(".transition.lock")?;
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

    pub fn pending(&self) -> Result<Option<(PathBuf, Manifest)>, String> {
        let root = self.root()?;
        if !root.exists() {
            return Ok(None);
        }
        let source = self.source()?;
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

    pub fn move_files(&self, dir: &Path, mut manifest: Manifest) -> Result<PathBuf, String> {
        if manifest.state == State::Moving {
            for name in &manifest.files {
                let from = self.0.parent().unwrap_or_else(|| Path::new(".")).join(name);
                let to = dir.join(name);
                move_one(&from, &to, name)?;
            }
            manifest.state = State::Ready;
            manifest.write(dir)?;
        }
        let final_dir = Estate(dir).ready()?;
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

    pub fn files(&self) -> Result<Vec<String>, String> {
        let name = self.filename()?;
        let mut files = vec![name.clone()];
        for suffix in ["-wal", "-shm", "-journal"] {
            let candidate = self.0.with_file_name(format!("{name}{suffix}"));
            if candidate.exists() {
                Estate(&candidate).refuse_non_file()?;
                files.push(format!("{name}{suffix}"));
            }
        }
        Ok(files)
    }

    pub fn refuse_orphans(&self) -> Result<(), String> {
        let name = self.filename()?;
        for suffix in ["-wal", "-shm", "-journal"] {
            let candidate = self.0.with_file_name(format!("{name}{suffix}"));
            if candidate.exists() {
                return Err(format!(
                    "orphan SQLite sidecar blocks fresh estate: {}",
                    candidate.display()
                ));
            }
        }
        Ok(())
    }

    pub fn refuse_non_file(&self) -> Result<(), String> {
        let kind = std::fs::symlink_metadata(self.0)
            .map_err(|error| error.to_string())?
            .file_type();
        if kind.is_file() && !kind.is_symlink() {
            Ok(())
        } else {
            Err(format!(
                "database transition target is not a file: {}",
                self.0.display()
            ))
        }
    }

    pub fn generation(&self) -> Result<PathBuf, String> {
        Ok(self.root()?.join(format!(
            ".moving-legacy-v{}-{}",
            signature::VERSION,
            santi_model::tag("q")
        )))
    }

    pub fn source(&self) -> Result<String, String> {
        let parent = self.0.parent().unwrap_or_else(|| Path::new("."));
        let parent = std::fs::canonicalize(parent).map_err(|error| error.to_string())?;
        Ok(parent.join(self.filename()?).display().to_string())
    }

    fn ready(&self) -> Result<PathBuf, String> {
        let name = self.filename()?;
        let name = name
            .strip_prefix(".moving-")
            .ok_or_else(|| format!("invalid moving quarantine generation {name}"))?;
        Ok(self.0.with_file_name(name))
    }

    fn root(&self) -> Result<PathBuf, String> {
        let name = self.filename()?;
        Ok(self.0.with_file_name(format!("{name}.quarantine")))
    }

    fn sibling(&self, suffix: &str) -> Result<PathBuf, String> {
        let name = self.filename()?;
        Ok(self.0.with_file_name(format!("{name}{suffix}")))
    }

    fn filename(&self) -> Result<String, String> {
        self.0
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_string)
            .ok_or_else(|| format!("database path has no UTF-8 filename: {}", self.0.display()))
    }
}

fn move_one(from: &Path, to: &Path, name: &str) -> Result<(), String> {
    match (from.exists(), to.exists()) {
        (true, false) => {
            Estate(from).refuse_non_file()?;
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

#[cfg(unix)]
fn sync(path: &Path) -> Result<(), String> {
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|error| error.to_string())
}

#[cfg(not(unix))]
fn sync(_: &Path) -> Result<(), String> {
    Ok(())
}
