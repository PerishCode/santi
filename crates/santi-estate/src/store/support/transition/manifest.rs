use serde::{Deserialize, Serialize};
#[cfg(unix)]
use std::fs::File;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

const FILE: &str = "transition.json";
const SCHEMA: &str = "santi.legacy-quarantine.v1";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum State {
    Moving,
    Ready,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Manifest {
    pub(super) schema: String,
    pub(super) state: State,
    pub(super) legacy_version: i64,
    pub(super) source: String,
    pub(super) created: String,
    pub(super) files: Vec<String>,
}

impl Manifest {
    pub(super) fn moving(source: String, files: Vec<String>) -> Self {
        Self {
            schema: SCHEMA.to_string(),
            state: State::Moving,
            legacy_version: super::signature::VERSION,
            source,
            created: santi_model::now(),
            files,
        }
    }

    pub(super) fn read(dir: &Path) -> Result<Self, String> {
        let path = dir.join(FILE);
        let bytes = std::fs::read(&path)
            .map_err(|error| format!("read transition manifest {}: {error}", path.display()))?;
        let manifest: Self = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub(super) fn write(&self, dir: &Path) -> Result<(), String> {
        self.validate()?;
        let path = dir.join(FILE);
        let temp = dir.join(format!("{FILE}.tmp"));
        let bytes = serde_json::to_vec_pretty(self).map_err(|error| error.to_string())?;
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temp)
            .map_err(|error| error.to_string())?;
        file.write_all(&bytes).map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        std::fs::rename(&temp, &path).map_err(|error| error.to_string())?;
        sync(dir)
    }

    fn validate(&self) -> Result<(), String> {
        if self.schema != SCHEMA || self.legacy_version != super::signature::VERSION {
            return Err("unknown legacy quarantine manifest".to_string());
        }
        if self.files.is_empty()
            || self.files.iter().any(|name| {
                let path = PathBuf::from(name);
                path.components().count() != 1 || path.file_name().is_none()
            })
        {
            return Err("invalid legacy quarantine file set".to_string());
        }
        Ok(())
    }
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
