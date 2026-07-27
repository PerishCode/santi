use anyhow::{Context, Result};

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
