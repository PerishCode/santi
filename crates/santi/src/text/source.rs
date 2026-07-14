use anyhow::{Context, Result};

struct Source {
    command: &'static str,
    label: &'static str,
    text: Option<String>,
    file: Option<String>,
    stdin: bool,
}

pub fn read_im_reply_text(
    text: Option<String>,
    file: Option<String>,
    stdin: bool,
) -> Result<String> {
    read_text_source(Source {
        command: "im reply",
        label: "reply",
        text,
        file,
        stdin,
    })
}

pub fn read_inbox_seed_text(
    text: Option<String>,
    file: Option<String>,
    stdin: bool,
) -> Result<String> {
    read_text_source(Source {
        command: "inbox seed",
        label: "seed",
        text,
        file,
        stdin,
    })
}

fn read_text_source(source: Source) -> Result<String> {
    match (source.text, source.file, source.stdin) {
        (Some(text), None, false) => Ok(text),
        (None, Some(path), false) => read_text_file(&path, source.label),
        (None, None, true) => read_text_file("-", source.label),
        (None, None, false) => {
            anyhow::bail!(
                "{} requires <text>, --file <path>, or --stdin",
                source.command
            )
        }
        _ => anyhow::bail!(
            "{} accepts exactly one of <text>, --file <path>, or --stdin",
            source.command
        ),
    }
}

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

pub(crate) fn read_summary_file(path: &str) -> Result<String> {
    read_text_file(path, "summary")
}
