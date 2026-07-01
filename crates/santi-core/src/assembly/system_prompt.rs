use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::{
    SESSION_WORKSPACE_URI, SOUL_WORKSPACE_URI, Strand, Timestamp, session_memory_uri,
    soul_memory_uri, timestamp_from_system_time,
};

const SANTI_CHANNEL: &str = "santi";

pub(crate) struct SystemPromptRequest<'a> {
    pub session_id: &'a str,
    pub strand: &'a Strand,
    pub soul_memory_path: PathBuf,
    pub session_memory_path: PathBuf,
}

pub(crate) fn render_system_prompt(request: SystemPromptRequest<'_>) -> Result<String, String> {
    let soul_memory = read_memory_material(&request.soul_memory_path)?;
    let session_memory = read_memory_material(&request.session_memory_path)?;
    let soul_source = soul_memory_uri();
    let session_source = session_memory_uri();

    Ok([
        "You are a distinct soul running inside this Santi instance.".to_string(),
        format!("{soul_source} will always be displayed in [santi-soul]."),
        format!("{session_source} will always be displayed in [santi-session]."),
        format!(
            "These files have no internal version history; save backups into {SOUL_WORKSPACE_URI} or {SESSION_WORKSPACE_URI} if needed."
        ),
        render_santi_system_description(),
        render_meta(request),
        render_memory_section("santi-soul", &soul_source, &soul_memory),
        render_memory_section("santi-session", &session_source, &session_memory),
    ]
    .join("\n\n"))
}

fn render_santi_system_description() -> String {
    [
        "<santi-system> blocks describe Santi runtime facts in this session.",
        "They are part of your context, not user speech or your natural-language reply.",
        "Read them as session facts about the workspace, runtime, or provider flow.",
    ]
    .join("\n")
}

fn render_meta(request: SystemPromptRequest<'_>) -> String {
    // No soul_name: a soul's name is not a runtime fact, it's part of the
    // soul's own memory (rendered in [santi-soul]). Dissolving soul_profile
    // dropped it here early; the fuller [santi-meta] slim (drop channel too) is
    // STEP 6.
    [
        "[santi-meta]".to_string(),
        format!("channel: {SANTI_CHANNEL}"),
        format!("soul_id: {}", request.strand.soul_id),
        format!("session_id: {}", request.session_id),
    ]
    .join("\n")
}

fn render_memory_section(name: &str, source: &str, memory: &MemoryMaterial) -> String {
    [
        format!("[{name}]"),
        format!("source: {source}"),
        format!(
            "updated_at: {}",
            memory.updated_at.as_deref().unwrap_or("null")
        ),
        "content:".to_string(),
        memory.content.clone(),
    ]
    .join("\n")
}

fn read_memory_material(path: &Path) -> Result<MemoryMaterial, String> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.to_string()),
    };
    let updated_at = match fs::metadata(path) {
        Ok(metadata) => metadata
            .modified()
            .ok()
            .and_then(|modified| timestamp_from_system_time(modified).ok()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.to_string()),
    };
    Ok(MemoryMaterial {
        content,
        updated_at,
    })
}

struct MemoryMaterial {
    content: String,
    updated_at: Option<Timestamp>,
}
