use std::{
    fmt, fs,
    path::{Component, Path, PathBuf},
    time::SystemTime,
};

const SCHEME: &str = "santi://";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bucket {
    pub soul: String,
    pub strand: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Uri {
    pub bucket: Bucket,
    pub key: String,
}

#[derive(Debug, Clone)]
pub struct Meta {
    pub uri: Uri,
    pub len: u64,
    pub modified: Option<SystemTime>,
}

#[derive(Debug, Clone)]
pub struct Payload {
    pub meta: Meta,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
}

struct Walk<'a> {
    bucket: &'a Bucket,
    root: &'a Path,
    prefix: &'a str,
    objects: &'a mut Vec<Meta>,
}

impl Bucket {
    pub fn new(soul: impl Into<String>, strand: impl Into<String>) -> Result<Self, String> {
        let bucket = Self {
            soul: soul.into(),
            strand: strand.into(),
        };
        plain("soul", &bucket.soul)?;
        plain("strand", &bucket.strand)?;
        Ok(bucket)
    }
}

impl Uri {
    pub fn new(bucket: Bucket, key: impl Into<String>) -> Result<Self, String> {
        let uri = Self {
            bucket,
            key: key.into(),
        };
        legal(&uri.key)?;
        Ok(uri)
    }

    pub fn parse(value: &str) -> Result<Self, String> {
        let raw = value
            .strip_prefix(SCHEME)
            .ok_or_else(|| "object uri must start with santi://".to_string())?;
        let mut parts = raw.splitn(3, '/');
        let soul = parts
            .next()
            .ok_or_else(|| "object uri missing soul id".to_string())?;
        let strand = parts
            .next()
            .ok_or_else(|| "object uri missing strand id".to_string())?;
        let key = parts
            .next()
            .ok_or_else(|| "object uri missing key".to_string())?;
        Self::new(Bucket::new(soul, strand)?, key)
    }

    pub fn http(&self) -> String {
        format!(
            "/api/v1/bucket/{}/{}/{}",
            escaped(&self.bucket.soul),
            escaped(&self.bucket.strand),
            coded(&self.key)
        )
    }
}

impl fmt::Display for Uri {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            out,
            "{SCHEME}{}/{}/{}",
            self.bucket.soul, self.bucket.strand, self.key
        )
    }
}

impl Store {
    pub fn new(runtime: impl Into<PathBuf>) -> Self {
        Self {
            root: runtime.into().join("buckets"),
        }
    }

    pub fn put(&self, uri: &Uri, bytes: &[u8]) -> Result<Meta, String> {
        let path = self.path(uri)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(&path, bytes).map_err(|error| error.to_string())?;
        self.head(uri)?
            .ok_or_else(|| "written object missing".to_string())
    }

    pub fn get(&self, uri: &Uri) -> Result<Option<Payload>, String> {
        let Some(meta) = self.head(uri)? else {
            return Ok(None);
        };
        let path = self.path(uri)?;
        let bytes = fs::read(path).map_err(|error| error.to_string())?;
        Ok(Some(Payload { meta, bytes }))
    }

    pub fn head(&self, uri: &Uri) -> Result<Option<Meta>, String> {
        let path = self.path(uri)?;
        let metadata = match fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.to_string()),
        };
        if !metadata.is_file() {
            return Err("object path is not a file".to_string());
        }
        Ok(Some(Meta {
            uri: uri.clone(),
            len: metadata.len(),
            modified: metadata.modified().ok(),
        }))
    }

    pub fn delete(&self, uri: &Uri) -> Result<bool, String> {
        let path = self.path(uri)?;
        match fs::remove_file(path) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.to_string()),
        }
    }

    pub fn list(&self, bucket: &Bucket, prefix: Option<&str>) -> Result<Vec<Meta>, String> {
        let prefix = prefix.unwrap_or("");
        if !prefix.is_empty() {
            legal(prefix)?;
        }
        let root = self.dir(bucket)?;
        let mut objects = Vec::new();
        if !root.exists() {
            return Ok(objects);
        }
        let mut walk = Walk {
            bucket,
            root: &root,
            prefix,
            objects: &mut objects,
        };
        self.gather(&mut walk, &root)?;
        objects.sort_by(|left, right| left.uri.key.cmp(&right.uri.key));
        Ok(objects)
    }

    fn gather(&self, walk: &mut Walk<'_>, dir: &Path) -> Result<(), String> {
        for entry in fs::read_dir(dir).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let path = entry.path();
            let metadata = entry.metadata().map_err(|error| error.to_string())?;
            if metadata.is_dir() {
                self.gather(walk, &path)?;
            } else if metadata.is_file() {
                let key = path
                    .strip_prefix(walk.root)
                    .map_err(|error| error.to_string())?
                    .to_string_lossy()
                    .replace('\\', "/");
                if !key.starts_with(walk.prefix) {
                    continue;
                }
                walk.objects.push(Meta {
                    uri: Uri::new(walk.bucket.clone(), key)?,
                    len: metadata.len(),
                    modified: metadata.modified().ok(),
                });
            }
        }
        Ok(())
    }

    fn dir(&self, bucket: &Bucket) -> Result<PathBuf, String> {
        plain("soul", &bucket.soul)?;
        plain("strand", &bucket.strand)?;
        Ok(self.root.join(&bucket.soul).join(&bucket.strand))
    }

    fn path(&self, uri: &Uri) -> Result<PathBuf, String> {
        legal(&uri.key)?;
        let path = self.dir(&uri.bucket)?.join(&uri.key);
        safe(Path::new(&uri.key))?;
        Ok(path)
    }
}

fn plain(label: &str, value: &str) -> Result<(), String> {
    if value.is_empty() || value == "." || value == ".." {
        return Err(format!("{label} must be a plain path segment"));
    }
    if value.contains('/') || value.contains('\\') {
        return Err(format!("{label} must not contain path separators"));
    }
    Ok(())
}

fn legal(value: &str) -> Result<(), String> {
    if value.is_empty() || value.starts_with('/') || value.contains('\\') {
        return Err("object key must be a relative forward-slash path".to_string());
    }
    if value.split('/').any(|segment| segment.is_empty()) {
        return Err("object key must not contain empty path segments".to_string());
    }
    safe(Path::new(value))
}

fn safe(path: &Path) -> Result<(), String> {
    for component in path.components() {
        match component {
            Component::Normal(value) if !value.is_empty() => {}
            _ => return Err("object key contains an unsafe path component".to_string()),
        }
    }
    Ok(())
}

fn coded(value: &str) -> String {
    value.split('/').map(escaped).collect::<Vec<_>>().join("/")
}

fn escaped(value: &str) -> String {
    let mut held = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            held.push(byte as char);
        } else {
            held.push_str(&format!("%{byte:02X}"));
        }
    }
    held
}
