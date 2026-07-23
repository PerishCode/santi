use std::path::{Component, Path, PathBuf};

pub const SOULSPACE: &str = "soul://";
pub const STRANDSPACE: &str = "strand://";
pub const MEMORY: &str = "MEMORY.md";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Root {
    Soul,
    Strand,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Uri {
    pub root: Root,
    pub path: PathBuf,
}

pub fn soulward() -> String {
    housed(SOULSPACE, MEMORY)
}

pub fn strandward() -> String {
    housed(STRANDSPACE, MEMORY)
}

pub fn housed(root: &str, path: &str) -> String {
    if path.is_empty() {
        return root.to_string();
    }
    format!("{root}{}", path.trim_start_matches('/'))
}

pub fn parsed(value: &str) -> Result<Uri, String> {
    if let Some(path) = value.strip_prefix(SOULSPACE) {
        return Ok(Uri {
            root: Root::Soul,
            path: safed(path, SOULSPACE)?,
        });
    }
    if let Some(path) = value.strip_prefix(STRANDSPACE) {
        return Ok(Uri {
            root: Root::Strand,
            path: safed(path, STRANDSPACE)?,
        });
    }
    if value.starts_with('@') {
        return Err(format!(
            "unsupported workspace alias: {value}; use {SOULSPACE} or {STRANDSPACE}"
        ));
    }
    if value.contains("://") {
        return Err(format!("unsupported workspace uri: {value}"));
    }
    Err(format!("cwd must use {SOULSPACE} or {STRANDSPACE}"))
}

fn safed(path: &str, root: &str) -> Result<PathBuf, String> {
    if path.is_empty() {
        return Ok(PathBuf::new());
    }
    let path = Path::new(path);
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(format!("workspace uri cannot escape {root}"));
    }
    Ok(path.to_path_buf())
}
