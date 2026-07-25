use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn sources(dir: &Path, held: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            sources(&path, held);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            held.push(path);
        }
    }
}

#[test]
fn codes_are_unique_and_lawful() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let crates = root.join("crates");
    let mut files = Vec::new();
    let Ok(entries) = std::fs::read_dir(&crates) else {
        panic!("crates dir missing");
    };
    for entry in entries.flatten() {
        sources(&entry.path().join("src"), &mut files);
    }
    let mut seen: HashMap<String, PathBuf> = HashMap::new();
    for file in files {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        for line in text.lines() {
            let Some(rest) = line.trim_start().strip_prefix("code: \"") else {
                continue;
            };
            let Some(code) = rest.split('"').next() else {
                continue;
            };
            assert!(
                code.chars()
                    .all(|held| held.is_ascii_lowercase() || held == '.' || held == '_'),
                "code {code} in {} must be lowercase dotted words",
                file.display()
            );
            if let Some(prior) = seen.insert(code.to_string(), file.clone()) {
                panic!(
                    "code {code} declared in both {} and {}",
                    prior.display(),
                    file.display()
                );
            }
        }
    }
    assert!(
        seen.len() >= 12,
        "expected the full catalog, found {}",
        seen.len()
    );
}
