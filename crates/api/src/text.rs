use anyhow::{Context, Result};

pub fn read(text: Option<String>, file: Option<String>, stdin: bool) -> Result<String> {
    match (text, file, stdin) {
        (Some(text), None, false) => Ok(text),
        (None, Some(path), false) => file_text(&path),
        (None, None, true) => file_text("-"),
        (None, None, false) => {
            anyhow::bail!("inbox seed requires <text>, --file <path>, or --stdin")
        }
        _ => anyhow::bail!("inbox seed accepts exactly one of <text>, --file <path>, or --stdin"),
    }
}

fn file_text(path: &str) -> Result<String> {
    if path == "-" {
        let mut held = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut held)
            .context("read seed from stdin")?;
        Ok(held)
    } else {
        std::fs::read_to_string(path).with_context(|| format!("read seed file {path}"))
    }
}
