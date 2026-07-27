use serde::Serialize;
use sha2::{Digest, Sha256};

use super::Draft;

const TIMEOUT: u64 = 60 * 60;
const TIMECAP: u64 = 24 * 60 * 60;
const OUTPUT: u64 = 16 * 1024 * 1024;
const OUTCAP: u64 = 64 * 1024 * 1024;
const DESCCAP: usize = 160;
const CMDCAP: usize = 64 * 1024;

#[derive(Serialize)]
pub(super) struct Normalized {
    pub description: String,
    pub command: String,
    pub cwd: Option<String>,
    #[serde(rename = "timeout_seconds")]
    pub timeout: u64,
    #[serde(rename = "output_limit_bytes")]
    pub output: u64,
    #[serde(rename = "remind_every_seconds")]
    pub remind: Option<u64>,
    #[serde(skip)]
    pub digest: String,
}

pub(super) fn normalize(draft: Draft) -> Result<Normalized, String> {
    let description = draft.description.trim().to_string();
    if description.is_empty() {
        return Err("job description must not be empty".to_string());
    }
    if description.len() > DESCCAP {
        return Err(format!("job description must not exceed {DESCCAP} bytes"));
    }
    let command = draft.command.trim().to_string();
    if command.is_empty() {
        return Err("job command must not be empty".to_string());
    }
    if command.len() > CMDCAP {
        return Err(format!("job command must not exceed {CMDCAP} bytes"));
    }
    let cwd = draft
        .cwd
        .map(|cwd| cwd.trim().to_string())
        .filter(|cwd| !cwd.is_empty());
    if let Some(cwd) = &cwd {
        crate::parsed(cwd)?;
    }
    let timeout = bounded("job timeout", draft.timeout.unwrap_or(TIMEOUT), TIMECAP)?;
    let output = bounded("job output limit", draft.output.unwrap_or(OUTPUT), OUTCAP)?;
    let remind = draft
        .remind
        .map(|value| bounded("job reminder interval", value, TIMECAP))
        .transpose()?;
    let mut normalized = Normalized {
        description,
        command,
        cwd,
        timeout,
        output,
        remind,
        digest: String::new(),
    };
    let encoded = serde_json::to_vec(&normalized).map_err(|error| error.to_string())?;
    normalized.digest = format!("{:x}", Sha256::digest(encoded));
    Ok(normalized)
}

fn bounded(name: &str, value: u64, maximum: u64) -> Result<u64, String> {
    if value == 0 {
        return Err(format!("{name} must be greater than zero"));
    }
    if value > maximum {
        return Err(format!("{name} must not exceed {maximum}"));
    }
    Ok(value)
}
