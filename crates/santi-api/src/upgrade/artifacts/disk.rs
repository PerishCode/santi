use super::*;

impl Store {
    pub(super) fn digest(&self, path: &Path) -> Result<String, String> {
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

    pub(crate) fn path_for(&self, artifact: &Artifact) -> Result<PathBuf, String> {
        if !is_sha256(&artifact.sha256) {
            return Err("package artifact sha256 is invalid".to_string());
        }
        Ok(self
            .packages_dir()
            .join(&artifact.sha256)
            .join(PACKAGE_FILE))
    }

    pub(super) fn packages_dir(&self) -> PathBuf {
        self.root.join("packages")
    }

    pub(super) fn installed_manifest_path(&self) -> PathBuf {
        self.root.join(INSTALLED_MANIFEST)
    }
}

pub(crate) fn write_json_atomic(path: &Path, value: &impl Serialize) -> Result<(), String> {
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
pub(super) fn replace_file(source: &Path, target: &Path) -> Result<(), String> {
    fs::rename(source, target)
        .map_err(|error| format!("atomically replace {}: {error}", target.display()))
}

#[cfg(windows)]
pub(super) fn replace_file(source: &Path, target: &Path) -> Result<(), String> {
    if target.exists() {
        fs::remove_file(target)
            .map_err(|error| format!("remove old {}: {error}", target.display()))?;
    }
    fs::rename(source, target).map_err(|error| format!("replace {}: {error}", target.display()))
}

#[cfg(target_os = "linux")]
pub(super) fn sync_directory(path: &Path) -> Result<(), String> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("sync directory {}: {error}", path.display()))
}

#[cfg(not(target_os = "linux"))]
pub(super) fn sync_directory(_path: &Path) -> Result<(), String> {
    Ok(())
}

pub(super) fn validate_identity(identity: Identity) -> Result<Identity, String> {
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

pub(super) fn validate_artifact_shape(artifact: &Artifact) -> Result<(), String> {
    validate_identity(Identity {
        package: artifact.package.clone(),
        version: artifact.version.clone(),
    })?;
    if !is_sha256(&artifact.sha256) {
        return Err("package artifact sha256 is invalid".to_string());
    }
    Ok(())
}

pub(super) fn require_matches_installed(
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

pub(super) fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

impl Store {
    pub(crate) fn verify(
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

    pub(crate) fn prune(&self, keep: &[&Artifact]) -> Result<(), String> {
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
}
