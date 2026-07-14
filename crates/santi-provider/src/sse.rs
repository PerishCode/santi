pub(crate) fn lines(buffer: &mut String) -> Vec<String> {
    let mut lines = Vec::new();
    while let Some(index) = buffer.find('\n') {
        let line = buffer[..index].trim_end_matches('\r').to_string();
        buffer.drain(..=index);
        lines.push(line);
    }
    lines
}

pub(crate) fn data(line: &str) -> Option<&str> {
    line.strip_prefix("data: ")
        .filter(|payload| *payload != "[DONE]")
}
