use std::fs::OpenOptions;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use santi_estate::{Bootstrap, Status};

struct Artifact(PathBuf);

pub async fn run(paths: &santi_api::config::Layout) -> Result<serde_json::Value, String> {
    let artifact = Artifact(paths.runtime.join("sudo"));
    let mut estate = Bootstrap::open(&paths.database).await?;
    let (sudo, created) = match artifact.load()? {
        Some(sudo) => (sudo, false),
        None => {
            if estate.status().await? != Status::Vacant {
                return Err("sudo custody is absent for an occupied estate".to_string());
            }
            let minted = estate.mint().await?;
            if artifact.keep(&minted)? {
                (minted, true)
            } else {
                let sudo = artifact
                    .load()?
                    .ok_or_else(|| "sudo custody was lost during creation".to_string())?;
                (sudo, false)
            }
        }
    };
    estate.seal(&sudo).await?;
    Ok(serde_json::json!({
        "database": paths.database,
        "sudo": artifact.0,
        "custody_created": created,
    }))
}

impl Artifact {
    fn load(&self) -> Result<Option<String>, String> {
        let bytes = match std::fs::read(&self.0) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(format!(
                    "cannot read sudo custody {}: {error}",
                    self.0.display()
                ));
            }
        };
        custody(&bytes).map(Some)
    }

    fn keep(&self, sudo: &str) -> Result<bool, String> {
        if let Some(parent) = self.0.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "cannot prepare sudo custody destination {}: {error}",
                    self.0.display()
                )
            })?;
        }
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = match options.open(&self.0) {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::AlreadyExists => return Ok(false),
            Err(error) => {
                return Err(format!(
                    "cannot create sudo custody {}: {error}",
                    self.0.display()
                ));
            }
        };
        file.write_all(sudo.as_bytes())
            .and_then(|_| file.sync_all())
            .map_err(|error| format!("cannot keep sudo custody {}: {error}", self.0.display()))?;
        sync(&self.0)?;
        Ok(true)
    }
}

fn custody(bytes: &[u8]) -> Result<String, String> {
    let text =
        std::str::from_utf8(bytes).map_err(|_| "sudo custody is not UTF-8 text".to_string())?;
    let sudo = text.trim();
    if sudo.len() != 64
        || !sudo
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("sudo custody is not a 64-character lowercase hexadecimal token".to_string());
    }
    Ok(sudo.to_string())
}

fn sync(path: &Path) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "sudo custody has no parent directory".to_string())?;
    std::fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| {
            format!(
                "cannot sync sudo custody directory {}: {error}",
                parent.display()
            )
        })
}
