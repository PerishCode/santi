use std::{
    fs,
    path::{Component, Path, PathBuf},
    time::SystemTime,
};

const SANTI_SCHEME: &str = "santi://";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectBucket {
    pub soul_id: String,
    pub strand_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectUri {
    pub bucket: ObjectBucket,
    pub key: String,
}

#[derive(Debug, Clone)]
pub struct ObjectMeta {
    pub uri: ObjectUri,
    pub len: u64,
    pub modified_at: Option<SystemTime>,
}

#[derive(Debug, Clone)]
pub struct ObjectPayload {
    pub meta: ObjectMeta,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct LocalObjectStore {
    root: PathBuf,
}

impl ObjectBucket {
    pub fn new(soul_id: impl Into<String>, strand_id: impl Into<String>) -> Result<Self, String> {
        let bucket = Self {
            soul_id: soul_id.into(),
            strand_id: strand_id.into(),
        };
        validate_segment("soul_id", &bucket.soul_id)?;
        validate_segment("strand_id", &bucket.strand_id)?;
        Ok(bucket)
    }
}

impl ObjectUri {
    pub fn new(bucket: ObjectBucket, key: impl Into<String>) -> Result<Self, String> {
        let uri = Self {
            bucket,
            key: key.into(),
        };
        validate_key(&uri.key)?;
        Ok(uri)
    }

    pub fn parse(value: &str) -> Result<Self, String> {
        let raw = value
            .strip_prefix(SANTI_SCHEME)
            .ok_or_else(|| "object uri must start with santi://".to_string())?;
        let mut parts = raw.splitn(3, '/');
        let soul_id = parts
            .next()
            .ok_or_else(|| "object uri missing soul id".to_string())?;
        let strand_id = parts
            .next()
            .ok_or_else(|| "object uri missing strand id".to_string())?;
        let key = parts
            .next()
            .ok_or_else(|| "object uri missing key".to_string())?;
        Self::new(ObjectBucket::new(soul_id, strand_id)?, key)
    }

    pub fn as_santi_uri(&self) -> String {
        format!(
            "{SANTI_SCHEME}{}/{}/{}",
            self.bucket.soul_id, self.bucket.strand_id, self.key
        )
    }

    pub fn as_http_path(&self) -> String {
        format!(
            "/api/v1/bucket/{}/{}/{}",
            percent_encode_path_component(&self.bucket.soul_id),
            percent_encode_path_component(&self.bucket.strand_id),
            percent_encode_key(&self.key)
        )
    }
}

impl LocalObjectStore {
    pub fn new(runtime_root: impl Into<PathBuf>) -> Self {
        Self {
            root: runtime_root.into().join("buckets"),
        }
    }

    pub fn put_object(&self, uri: &ObjectUri, bytes: &[u8]) -> Result<ObjectMeta, String> {
        let path = self.object_path(uri)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(&path, bytes).map_err(|error| error.to_string())?;
        self.head_object(uri)?
            .ok_or_else(|| "written object missing".to_string())
    }

    pub fn get_object(&self, uri: &ObjectUri) -> Result<Option<ObjectPayload>, String> {
        let Some(meta) = self.head_object(uri)? else {
            return Ok(None);
        };
        let path = self.object_path(uri)?;
        let bytes = fs::read(path).map_err(|error| error.to_string())?;
        Ok(Some(ObjectPayload { meta, bytes }))
    }

    pub fn head_object(&self, uri: &ObjectUri) -> Result<Option<ObjectMeta>, String> {
        let path = self.object_path(uri)?;
        let metadata = match fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.to_string()),
        };
        if !metadata.is_file() {
            return Err("object path is not a file".to_string());
        }
        Ok(Some(ObjectMeta {
            uri: uri.clone(),
            len: metadata.len(),
            modified_at: metadata.modified().ok(),
        }))
    }

    pub fn delete_object(&self, uri: &ObjectUri) -> Result<bool, String> {
        let path = self.object_path(uri)?;
        match fs::remove_file(path) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.to_string()),
        }
    }

    pub fn list_objects(
        &self,
        bucket: &ObjectBucket,
        prefix: Option<&str>,
    ) -> Result<Vec<ObjectMeta>, String> {
        let prefix = prefix.unwrap_or("");
        if !prefix.is_empty() {
            validate_key(prefix)?;
        }
        let bucket_root = self.bucket_path(bucket)?;
        let mut objects = Vec::new();
        if !bucket_root.exists() {
            return Ok(objects);
        }
        self.collect_objects(bucket, &bucket_root, &bucket_root, prefix, &mut objects)?;
        objects.sort_by(|left, right| left.uri.key.cmp(&right.uri.key));
        Ok(objects)
    }

    fn collect_objects(
        &self,
        bucket: &ObjectBucket,
        bucket_root: &Path,
        dir: &Path,
        prefix: &str,
        objects: &mut Vec<ObjectMeta>,
    ) -> Result<(), String> {
        for entry in fs::read_dir(dir).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let path = entry.path();
            let metadata = entry.metadata().map_err(|error| error.to_string())?;
            if metadata.is_dir() {
                self.collect_objects(bucket, bucket_root, &path, prefix, objects)?;
            } else if metadata.is_file() {
                let key = path
                    .strip_prefix(bucket_root)
                    .map_err(|error| error.to_string())?
                    .to_string_lossy()
                    .replace('\\', "/");
                if !key.starts_with(prefix) {
                    continue;
                }
                objects.push(ObjectMeta {
                    uri: ObjectUri::new(bucket.clone(), key)?,
                    len: metadata.len(),
                    modified_at: metadata.modified().ok(),
                });
            }
        }
        Ok(())
    }

    fn bucket_path(&self, bucket: &ObjectBucket) -> Result<PathBuf, String> {
        validate_segment("soul_id", &bucket.soul_id)?;
        validate_segment("strand_id", &bucket.strand_id)?;
        Ok(self.root.join(&bucket.soul_id).join(&bucket.strand_id))
    }

    fn object_path(&self, uri: &ObjectUri) -> Result<PathBuf, String> {
        validate_key(&uri.key)?;
        let path = self.bucket_path(&uri.bucket)?.join(&uri.key);
        ensure_relative_components(Path::new(&uri.key))?;
        Ok(path)
    }
}

fn validate_segment(label: &str, value: &str) -> Result<(), String> {
    if value.is_empty() || value == "." || value == ".." {
        return Err(format!("{label} must be a plain path segment"));
    }
    if value.contains('/') || value.contains('\\') {
        return Err(format!("{label} must not contain path separators"));
    }
    Ok(())
}

fn validate_key(value: &str) -> Result<(), String> {
    if value.is_empty() || value.starts_with('/') || value.contains('\\') {
        return Err("object key must be a relative forward-slash path".to_string());
    }
    if value.split('/').any(|segment| segment.is_empty()) {
        return Err("object key must not contain empty path segments".to_string());
    }
    ensure_relative_components(Path::new(value))
}

fn ensure_relative_components(path: &Path) -> Result<(), String> {
    for component in path.components() {
        match component {
            Component::Normal(value) if !value.is_empty() => {}
            _ => return Err("object key contains an unsafe path component".to_string()),
        }
    }
    Ok(())
}

fn percent_encode_key(value: &str) -> String {
    value
        .split('/')
        .map(percent_encode_path_component)
        .collect::<Vec<_>>()
        .join("/")
}

fn percent_encode_path_component(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}
