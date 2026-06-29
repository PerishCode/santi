use std::path::{Component, Path, PathBuf};

pub const SOUL_WORKSPACE_URI: &str = "soul://";
pub const SESSION_WORKSPACE_URI: &str = "session://";
pub const MEMORY_FILE: &str = "MEMORY.md";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceRoot {
    Soul,
    Session,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceUri {
    pub root: WorkspaceRoot,
    pub path: PathBuf,
}

pub fn soul_memory_uri() -> String {
    workspace_uri(SOUL_WORKSPACE_URI, MEMORY_FILE)
}

pub fn session_memory_uri() -> String {
    workspace_uri(SESSION_WORKSPACE_URI, MEMORY_FILE)
}

pub fn workspace_uri(root: &str, path: &str) -> String {
    if path.is_empty() {
        return root.to_string();
    }
    format!("{root}{}", path.trim_start_matches('/'))
}

pub fn parse_workspace_uri(value: &str) -> Result<WorkspaceUri, String> {
    if let Some(path) = value.strip_prefix(SOUL_WORKSPACE_URI) {
        return Ok(WorkspaceUri {
            root: WorkspaceRoot::Soul,
            path: safe_relative_path(path, SOUL_WORKSPACE_URI)?,
        });
    }
    if let Some(path) = value.strip_prefix(SESSION_WORKSPACE_URI) {
        return Ok(WorkspaceUri {
            root: WorkspaceRoot::Session,
            path: safe_relative_path(path, SESSION_WORKSPACE_URI)?,
        });
    }
    if value.starts_with('@') {
        return Err(format!(
            "unsupported workspace alias: {value}; use {SOUL_WORKSPACE_URI} or {SESSION_WORKSPACE_URI}"
        ));
    }
    if value.contains("://") {
        return Err(format!("unsupported workspace uri: {value}"));
    }
    Err(format!(
        "cwd must use {SOUL_WORKSPACE_URI} or {SESSION_WORKSPACE_URI}"
    ))
}

fn safe_relative_path(path: &str, root: &str) -> Result<PathBuf, String> {
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
