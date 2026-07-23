use std::fs::{self, File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

const ARTIFACT_PROTOCOL_VERSION: u32 = 1;
const PACKAGE_NAME: &str = "santi";
const INSTALLED_MANIFEST: &str = "installed-package.json";
const PACKAGE_FILE: &str = "santi.deb";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Artifact {
    pub package: String,
    pub version: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Identity {
    pub package: String,
    pub version: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Manifest {
    protocol_version: u32,
    artifact: Artifact,
}

pub(super) trait Probe {
    fn inspect_deb(&self, path: &Path) -> Result<Identity, String>;
    fn installed(&self) -> Result<Identity, String>;
}

pub(super) struct Dpkg;

#[derive(Debug, Clone)]
pub(super) struct Store {
    root: PathBuf,
}

impl Store {
    pub(super) fn new(runtime: &Path) -> Self {
        Self {
            root: runtime.join("upgrade"),
        }
    }

    pub(super) fn stage(&self, source: &Path, probe: &impl Probe) -> Result<Artifact, String> {
        if !source.is_file() {
            return Err(format!(
                "package artifact is not a readable file: {}",
                source.display()
            ));
        }
        let packages = self.packages_dir();
        fs::create_dir_all(&packages)
            .map_err(|error| format!("create package artifact directory: {error}"))?;
        let temporary = packages.join(format!(".stage-{}.tmp", Uuid::new_v4().simple()));
        let result = self.stage_at(source, &temporary, probe);
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    fn stage_at(
        &self,
        source: &Path,
        temporary: &Path,
        probe: &impl Probe,
    ) -> Result<Artifact, String> {
        let mut input = File::open(source)
            .map_err(|error| format!("open package artifact {}: {error}", source.display()))?;
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(temporary)
            .map_err(|error| format!("create staged package artifact: {error}"))?;
        let mut digest = Sha256::new();
        let mut bytes = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = input
                .read(&mut buffer)
                .map_err(|error| format!("read package artifact: {error}"))?;
            if read == 0 {
                break;
            }
            output
                .write_all(&buffer[..read])
                .map_err(|error| format!("write staged package artifact: {error}"))?;
            digest.update(&buffer[..read]);
            bytes += read as u64;
        }
        output
            .sync_all()
            .map_err(|error| format!("sync staged package artifact: {error}"))?;
        drop(output);

        let identity = validate_identity(probe.inspect_deb(temporary)?)?;
        let artifact = Artifact {
            package: identity.package,
            version: identity.version,
            sha256: hex::encode(digest.finalize()),
            bytes,
        };
        let target = self.path_for(&artifact)?;
        let parent = target
            .parent()
            .ok_or_else(|| "package artifact target has no parent".to_string())?;
        let packages = parent
            .parent()
            .ok_or_else(|| "package artifact directory has no parent".to_string())?;
        fs::create_dir_all(parent)
            .map_err(|error| format!("create content-addressed package directory: {error}"))?;
        sync_directory(packages)?;
        if target.exists() {
            fs::remove_file(temporary)
                .map_err(|error| format!("remove duplicate staged package: {error}"))?;
        } else {
            fs::rename(temporary, &target)
                .map_err(|error| format!("commit staged package artifact: {error}"))?;
            sync_directory(parent)?;
        }
        self.verify(&artifact, probe)?;
        Ok(artifact)
    }

    pub(super) fn resolve_previous(
        &self,
        supplied: Option<&Path>,
        probe: &impl Probe,
    ) -> Result<Artifact, String> {
        let installed = validate_identity(probe.installed()?)?;
        let durable = self.load_installed(probe)?;
        let supplied = supplied.map(|path| self.stage(path, probe)).transpose()?;
        if let Some(artifact) = &supplied {
            require_matches_installed(artifact, &installed, "supplied previous package")?;
        }

        match (durable, supplied) {
            (Some(durable), supplied) => {
                require_matches_installed(&durable, &installed, "durable previous package")?;
                if let Some(supplied) = supplied
                    && supplied.sha256 != durable.sha256
                {
                    return Err(format!(
                        "supplied previous package differs from durable installed artifact: durable={} supplied={}",
                        durable.sha256, supplied.sha256
                    ));
                }
                Ok(durable)
            }
            (None, Some(supplied)) => {
                self.commit_installed(&supplied, probe)?;
                Ok(supplied)
            }
            (None, None) => Err(
                "no durable previous package artifact; bootstrap once with --previous-deb or SANTI_PREVIOUS_DEB"
                    .to_string(),
            ),
        }
    }

    pub(super) fn load_installed(&self, probe: &impl Probe) -> Result<Option<Artifact>, String> {
        let raw = match fs::read(self.installed_manifest_path()) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("read installed package manifest: {error}")),
        };
        let manifest: Manifest = serde_json::from_slice(&raw)
            .map_err(|error| format!("decode installed package manifest: {error}"))?;
        if manifest.protocol_version != ARTIFACT_PROTOCOL_VERSION {
            return Err(format!(
                "unsupported installed package manifest protocol: {}",
                manifest.protocol_version
            ));
        }
        self.verify(&manifest.artifact, probe)?;
        Ok(Some(manifest.artifact))
    }

    pub(super) fn commit_installed(
        &self,
        artifact: &Artifact,
        probe: &impl Probe,
    ) -> Result<(), String> {
        self.verify(artifact, probe)?;
        let installed = validate_identity(probe.installed()?)?;
        require_matches_installed(artifact, &installed, "retained package")?;
        let manifest = Manifest {
            protocol_version: ARTIFACT_PROTOCOL_VERSION,
            artifact: artifact.clone(),
        };
        write_json_atomic(&self.installed_manifest_path(), &manifest)
    }
}

#[path = "artifacts/disk.rs"]
mod disk;
#[path = "artifacts/probe.rs"]
mod probe;
pub(crate) use disk::write_json_atomic;
use disk::*;
