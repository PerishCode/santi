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
#[cfg(target_os = "windows")]
const LEASE: &str = "sidecar.lock";

pub(super) struct Directory<'a>(&'a Path);

struct Artifact<'a>(&'a Path);

pub(super) fn directory(path: &Path) -> Directory<'_> {
    Directory(path)
}

impl Directory<'_> {
    pub fn prepare(&self) -> Result<(), String> {
        fs::create_dir_all(self.0).map_err(|error| error.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(self.0, fs::Permissions::from_mode(0o700))
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    pub fn specify(&self, requested: &Spec) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(requested).map_err(|error| error.to_string())?;
        let path = self.0.join(SPEC);
        if path.exists() {
            let existing = self.spec()?;
            return if existing.matches(requested) {
                Ok(())
            } else {
                Err("job execution spec conflicts with its retained stamp".to_string())
            };
        }
        Artifact(&path).replace(&bytes)
    }

    pub fn spec(&self) -> Result<Spec, String> {
        let bytes = fs::read(self.0.join(SPEC)).map_err(|error| error.to_string())?;
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())
    }

    pub fn state(&self) -> Result<Option<Snapshot>, String> {
        let path = self.0.join(STATE);
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

    pub fn advance(&self, phase: Phase) -> Result<(), String> {
        if let Some(current) = self.state()? {
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
        Artifact(&self.0.join(STATE)).replace(&bytes)
    }

    pub fn terminal(&self) -> Result<Option<Record>, String> {
        for name in [RESULT, TERMINAL] {
            match fs::read(self.0.join(name)) {
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

    pub fn finish(&self, terminal: &Record) -> Result<(), String> {
        if self.terminal()?.is_some() {
            return Ok(());
        }
        let bytes = serde_json::to_vec_pretty(terminal).map_err(|error| error.to_string())?;
        Artifact(&self.0.join(RESULT)).once(&bytes)
    }

    pub fn mark(&self, name: &str) -> Result<(), String> {
        Artifact(&self.0.join(name)).once(b"1\n")
    }

    pub fn log(&self, name: &str) -> Result<File, String> {
        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options
            .open(self.0.join(name))
            .map_err(|error| error.to_string())
    }

    #[cfg(target_os = "windows")]
    pub fn lease(&self) -> Result<File, String> {
        use fs2::FileExt;

        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(self.0.join(LEASE))
            .map_err(|error| error.to_string())?;
        file.try_lock_exclusive()
            .map_err(|error| format!("job sidecar lease is held: {error}"))?;
        Ok(file)
    }

    #[cfg(target_os = "windows")]
    pub fn active(&self) -> Result<bool, String> {
        use fs2::FileExt;

        let file = match OpenOptions::new()
            .read(true)
            .write(true)
            .open(self.0.join(LEASE))
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.to_string()),
        };
        match file.try_lock_exclusive() {
            Ok(()) => {
                fs2::FileExt::unlock(&file).map_err(|error| error.to_string())?;
                Ok(false)
            }
            Err(error) if contended(&error) => Ok(true),
            Err(error) => Err(error.to_string()),
        }
    }
}

#[cfg(target_os = "windows")]
fn contended(error: &std::io::Error) -> bool {
    const ERROR_LOCK_VIOLATION: i32 = 33;

    error.kind() == std::io::ErrorKind::WouldBlock
        || error.raw_os_error() == Some(ERROR_LOCK_VIOLATION)
}

#[cfg(target_os = "macos")]
impl Directory<'_> {
    pub fn artifact(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        Artifact(&self.0.join(name)).replace(bytes)
    }
}

impl Artifact<'_> {
    fn replace(&self, bytes: &[u8]) -> Result<(), String> {
        let temporary = self
            .0
            .with_extension(format!("tmp.{}", uuid::Uuid::new_v4().simple()));
        let mut file = Artifact(&temporary).create()?;
        file.write_all(bytes).map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        replace_file(&temporary, self.0)?;
        self.sync()
    }

    fn once(&self, bytes: &[u8]) -> Result<(), String> {
        if self.0.exists() {
            return Ok(());
        }
        let temporary = self
            .0
            .with_extension(format!("candidate.{}", uuid::Uuid::new_v4().simple()));
        let mut file = Artifact(&temporary).create()?;
        file.write_all(bytes).map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        match fs::hard_link(&temporary, self.0) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                let _ = fs::remove_file(&temporary);
                return Err(error.to_string());
            }
        }
        fs::remove_file(&temporary).map_err(|error| error.to_string())?;
        self.sync()
    }

    fn create(&self) -> Result<File, String> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options.open(self.0).map_err(|error| error.to_string())
    }
}

#[cfg(unix)]
fn replace_file(temporary: &Path, path: &Path) -> Result<(), String> {
    fs::rename(temporary, path).map_err(|error| error.to_string())
}

#[cfg(target_os = "windows")]
fn replace_file(temporary: &Path, path: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let source = temporary
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let target = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let moved = unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(())
    }
}

#[cfg(unix)]
impl Artifact<'_> {
    fn sync(&self) -> Result<(), String> {
        let parent = self
            .0
            .parent()
            .ok_or_else(|| "job artifact has no parent directory".to_string())?;
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| error.to_string())
    }
}

#[cfg(not(unix))]
impl Artifact<'_> {
    fn sync(&self) -> Result<(), String> {
        let _ = self;
        Ok(())
    }
}
