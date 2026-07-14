use std::fs::{self, File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::Command;

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

impl Probe for Dpkg {
    fn inspect_deb(&self, path: &Path) -> Result<Identity, String> {
        Ok(Identity {
            package: deb_field(path, "Package")?,
            version: deb_field(path, "Version")?,
        })
    }

    fn installed(&self) -> Result<Identity, String> {
        let output = Command::new("dpkg-query")
            .args(["-W", "-f=${Package}\t${Version}", PACKAGE_NAME])
            .output()
            .map_err(|error| format!("query installed santi package: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "query installed santi package failed with {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        let value = String::from_utf8(output.stdout)
            .map_err(|error| format!("installed package identity is not UTF-8: {error}"))?;
        let (package, version) = value
            .trim()
            .split_once('\t')
            .ok_or_else(|| "installed package identity has an invalid shape".to_string())?;
        validate_identity(Identity {
            package: package.to_string(),
            version: version.to_string(),
        })
    }
}

fn deb_field(path: &Path, field: &str) -> Result<String, String> {
    let output = Command::new("dpkg-deb")
        .arg("--field")
        .arg(path)
        .arg(field)
        .output()
        .map_err(|error| format!("inspect {} field {field}: {error}", path.display()))?;
    if !output.status.success() {
        return Err(format!(
            "inspect {} field {field} failed with {}: {}",
            path.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let value = String::from_utf8(output.stdout)
        .map_err(|error| format!("{field} field is not UTF-8: {error}"))?;
    let value = value.trim();
    if value.is_empty() {
        Err(format!("{} has an empty {field} field", path.display()))
    } else {
        Ok(value.to_string())
    }
}

#[derive(Debug, Clone)]
pub(super) struct Store {
    root: PathBuf,
}

impl Store {
    pub(super) fn new(runtime_root: &Path) -> Self {
        Self {
            root: runtime_root.join("upgrade"),
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

    pub(super) fn verify(
        &self,
        artifact: &Artifact,
        probe: &impl Probe,
    ) -> Result<PathBuf, String> {
        validate_artifact_shape(artifact)?;
        let path = self.path_for(artifact)?;
        let metadata = fs::metadata(&path).map_err(|error| {
            format!("read retained package metadata {}: {error}", path.display())
        })?;
        if !metadata.is_file() {
            return Err(format!(
                "retained package is not a file: {}",
                path.display()
            ));
        }
        if metadata.len() != artifact.bytes {
            return Err(format!(
                "retained package size mismatch: expected={} actual={}",
                artifact.bytes,
                metadata.len()
            ));
        }
        let actual = self.digest(&path)?;
        if actual != artifact.sha256 {
            return Err(format!(
                "retained package checksum mismatch: expected={} actual={actual}",
                artifact.sha256
            ));
        }
        let identity = validate_identity(probe.inspect_deb(&path)?)?;
        if identity.package != artifact.package || identity.version != artifact.version {
            return Err(format!(
                "retained package identity mismatch: expected={} {} actual={} {}",
                artifact.package, artifact.version, identity.package, identity.version
            ));
        }
        Ok(path)
    }

    pub(super) fn prune(&self, keep: &[&Artifact]) -> Result<(), String> {
        let packages = self.packages_dir();
        let entries = match fs::read_dir(&packages) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(format!("read retained package directory: {error}")),
        };
        for entry in entries {
            let entry = entry.map_err(|error| format!("read retained package entry: {error}"))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            if name.starts_with(".stage-") && name.ends_with(".tmp") {
                fs::remove_file(&path)
                    .map_err(|error| format!("remove interrupted package staging file: {error}"))?;
                continue;
            }
            if is_sha256(&name) && !keep.iter().any(|artifact| artifact.sha256 == name) {
                fs::remove_dir_all(&path)
                    .map_err(|error| format!("remove unreferenced package artifact: {error}"))?;
            }
        }
        Ok(())
    }

    fn digest(&self, path: &Path) -> Result<String, String> {
        let mut file = File::open(path)
            .map_err(|error| format!("open retained package {}: {error}", path.display()))?;
        let mut digest = Sha256::new();
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = file
                .read(&mut buffer)
                .map_err(|error| format!("read retained package {}: {error}", path.display()))?;
            if read == 0 {
                break;
            }
            digest.update(&buffer[..read]);
        }
        Ok(hex::encode(digest.finalize()))
    }

    pub(super) fn path_for(&self, artifact: &Artifact) -> Result<PathBuf, String> {
        if !is_sha256(&artifact.sha256) {
            return Err("package artifact sha256 is invalid".to_string());
        }
        Ok(self
            .packages_dir()
            .join(&artifact.sha256)
            .join(PACKAGE_FILE))
    }

    fn packages_dir(&self) -> PathBuf {
        self.root.join("packages")
    }

    fn installed_manifest_path(&self) -> PathBuf {
        self.root.join(INSTALLED_MANIFEST)
    }
}

pub(super) fn write_json_atomic(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("create {} parent: {error}", path.display()))?;
    let payload =
        serde_json::to_vec(value).map_err(|error| format!("encode {}: {error}", path.display()))?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("manifest"),
        Uuid::new_v4().simple()
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| format!("create {}: {error}", temporary.display()))?;
        file.write_all(&payload)
            .map_err(|error| format!("write {}: {error}", temporary.display()))?;
        file.sync_all()
            .map_err(|error| format!("sync {}: {error}", temporary.display()))?;
        drop(file);
        replace_file(&temporary, path)?;
        sync_directory(parent)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(not(windows))]
fn replace_file(source: &Path, target: &Path) -> Result<(), String> {
    fs::rename(source, target)
        .map_err(|error| format!("atomically replace {}: {error}", target.display()))
}

#[cfg(windows)]
fn replace_file(source: &Path, target: &Path) -> Result<(), String> {
    if target.exists() {
        fs::remove_file(target)
            .map_err(|error| format!("remove old {}: {error}", target.display()))?;
    }
    fs::rename(source, target).map_err(|error| format!("replace {}: {error}", target.display()))
}

#[cfg(target_os = "linux")]
fn sync_directory(path: &Path) -> Result<(), String> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("sync directory {}: {error}", path.display()))
}

#[cfg(not(target_os = "linux"))]
fn sync_directory(_path: &Path) -> Result<(), String> {
    Ok(())
}

fn validate_identity(identity: Identity) -> Result<Identity, String> {
    if identity.package != PACKAGE_NAME {
        return Err(format!(
            "package identity mismatch: expected={PACKAGE_NAME} actual={}",
            identity.package
        ));
    }
    if identity.version.trim().is_empty() {
        return Err("package version is empty".to_string());
    }
    Ok(identity)
}

fn validate_artifact_shape(artifact: &Artifact) -> Result<(), String> {
    validate_identity(Identity {
        package: artifact.package.clone(),
        version: artifact.version.clone(),
    })?;
    if !is_sha256(&artifact.sha256) {
        return Err("package artifact sha256 is invalid".to_string());
    }
    Ok(())
}

fn require_matches_installed(
    artifact: &Artifact,
    installed: &Identity,
    label: &str,
) -> Result<(), String> {
    if artifact.package == installed.package && artifact.version == installed.version {
        Ok(())
    } else {
        Err(format!(
            "{label} does not match installed package: installed={} {} artifact={} {}",
            installed.package, installed.version, artifact.package, artifact.version
        ))
    }
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}
