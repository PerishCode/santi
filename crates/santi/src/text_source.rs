use anyhow::{Context, Result};

/// Resolve the soul's IM reply body from exactly one supported source.
pub fn read_im_reply_text(
    text: Option<String>,
    file: Option<String>,
    stdin: bool,
) -> Result<String> {
    read_text_source("im reply", "reply", text, file, stdin)
}

/// Resolve an offline inbox seed body from exactly one supported source.
pub fn read_inbox_seed_text(
    text: Option<String>,
    file: Option<String>,
    stdin: bool,
) -> Result<String> {
    read_text_source("inbox seed", "seed", text, file, stdin)
}

/// Resolve a command text body from exactly one supported source.
fn read_text_source(
    command: &str,
    label: &str,
    text: Option<String>,
    file: Option<String>,
    stdin: bool,
) -> Result<String> {
    match (text, file, stdin) {
        (Some(text), None, false) => Ok(text),
        (None, Some(path), false) => read_text_file(&path, label),
        (None, None, true) => read_text_file("-", label),
        (None, None, false) => {
            anyhow::bail!("{command} requires <text>, --file <path>, or --stdin")
        }
        _ => anyhow::bail!("{command} accepts exactly one of <text>, --file <path>, or --stdin"),
    }
}

/// Read a text value from a file, or stdin when the path is `-`.
fn read_text_file(path: &str, label: &str) -> Result<String> {
    if path == "-" {
        let mut buf = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf)
            .with_context(|| format!("read {label} from stdin"))?;
        Ok(buf)
    } else {
        std::fs::read_to_string(path).with_context(|| format!("read {label} file {path}"))
    }
}

/// Read a compact summary from a file, or stdin when the path is `-`.
pub(crate) fn read_summary_file(path: &str) -> Result<String> {
    read_text_file(path, "summary")
}
