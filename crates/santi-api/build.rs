use sha2::{Digest, Sha256};
use std::{fs, path::Path, path::PathBuf};

const ALGORITHM: &str = "sha256-tree-v1";
const SCHEMA: &str = "santi.web_dist.v1";
const RECIPE: [&str; 8] = [
    "web/index.html",
    "web/vite.config.ts",
    "web/tsconfig.json",
    "web/package.json",
    "web/pnpm-workspace.yaml",
    "web/pnpm-lock.yaml",
    "web/.node-version",
    ".runseal/lib/web/manifest.ts",
];

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    for path in [
        "web/src",
        "web/public",
        "web/dist",
        "web/dist.manifest",
        ".runseal/lib/web/vectors.json",
    ] {
        println!("cargo:rerun-if-changed={}", root.join(path).display());
    }
    for path in RECIPE {
        println!("cargo:rerun-if-changed={}", root.join(path).display());
    }
    vectors(&root);
    let manifest = read(&root.join("web/dist.manifest"));
    let parsed: serde_json::Value = serde_json::from_str(&manifest)
        .unwrap_or_else(|error| quit(&format!("web/dist.manifest is not valid JSON: {error}")));
    field(&parsed, "schema", SCHEMA);
    field(&parsed, "algorithm", ALGORITHM);
    pins(&root, &parsed);
    let mut inputs = Vec::new();
    for source in ["web/src", "web/public"] {
        let base = root.join(source);
        if base.exists() {
            walk(&base, source, &mut inputs);
        }
    }
    for path in RECIPE {
        inputs.push((path.to_string(), bytes(&root.join(path))));
    }
    compare(&parsed, "inputs", &digest(inputs));
    let dist = root.join("web/dist");
    if !dist.join("index.html").exists() {
        quit(
            "web/dist has no index.html; run: pnpm --dir web build (guard generates dist + manifest)",
        );
    }
    let mut output = Vec::new();
    walk(&dist, "", &mut output);
    compare(&parsed, "output", &digest(output));
}

fn walk(base: &Path, prefix: &str, entries: &mut Vec<(String, Vec<u8>)>) {
    let listing = fs::read_dir(base)
        .unwrap_or_else(|error| quit(&format!("cannot read {}: {error}", base.display())));
    for entry in listing {
        let entry = entry.unwrap_or_else(|error| quit(&format!("directory entry: {error}")));
        let kind = entry
            .file_type()
            .unwrap_or_else(|error| quit(&format!("file type: {error}")));
        let name = entry.file_name().to_string_lossy().to_string();
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        if kind.is_symlink() || (!kind.is_dir() && !kind.is_file()) {
            quit(&format!(
                "{path}: symlinks and non-regular entries are rejected"
            ));
        }
        if kind.is_dir() {
            walk(&entry.path(), &path, entries);
        } else {
            entries.push((path, bytes(&entry.path())));
        }
    }
}

fn digest(mut entries: Vec<(String, Vec<u8>)>) -> String {
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    let mut lines = String::new();
    for (path, body) in &entries {
        lines.push_str(&format!("{}  {}\n", hex(body), path));
    }
    hex(lines.as_bytes())
}

fn hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn vectors(root: &Path) {
    let text = read(&root.join(".runseal/lib/web/vectors.json"));
    let parsed: serde_json::Value = serde_json::from_str(&text)
        .unwrap_or_else(|error| quit(&format!("vectors.json is not valid JSON: {error}")));
    let list = parsed["vectors"]
        .as_array()
        .unwrap_or_else(|| quit("vectors.json has no vectors array"));
    for vector in list {
        let entries: Vec<(String, Vec<u8>)> = vector["files"]
            .as_array()
            .unwrap_or_else(|| quit("vector without files"))
            .iter()
            .map(|file| {
                (
                    file["path"].as_str().unwrap_or_default().to_string(),
                    file["text"]
                        .as_str()
                        .unwrap_or_default()
                        .as_bytes()
                        .to_vec(),
                )
            })
            .collect();
        let wanted = vector["digest"].as_str().unwrap_or_default();
        let name = vector["name"].as_str().unwrap_or_default();
        let got = digest(entries);
        if got != wanted {
            quit(&format!(
                "cross-language digest conformance FAILED on vector \"{name}\": rust {got} != shared {wanted}"
            ));
        }
    }
}

fn pins(root: &Path, parsed: &serde_json::Value) {
    let node = read(&root.join("web/.node-version")).trim().to_string();
    let package = read(&root.join("web/package.json"));
    let recorded = parsed["node"].as_str().unwrap_or_default();
    if recorded != format!("v{node}") {
        quit(&format!(
            "dist.manifest node {recorded} does not match the pin v{node}"
        ));
    }
    let pnpm = parsed["pnpm"].as_str().unwrap_or_default();
    if !package.contains(&format!("\"packageManager\": \"pnpm@{pnpm}\"")) {
        quit(&format!(
            "dist.manifest pnpm {pnpm} does not match web/package.json packageManager"
        ));
    }
}

fn field(parsed: &serde_json::Value, name: &str, wanted: &str) {
    let got = parsed[name].as_str().unwrap_or_default();
    if got != wanted {
        quit(&format!(
            "dist.manifest {name} \"{got}\" != required \"{wanted}\""
        ));
    }
}

fn compare(parsed: &serde_json::Value, name: &str, got: &str) {
    let wanted = parsed[name].as_str().unwrap_or_default();
    if wanted != got {
        quit(&format!(
            "web build is STALE: recomputed {name} digest {got} != dist.manifest {wanted}; run: pnpm --dir web build (then the guard regenerates dist.manifest)"
        ));
    }
}

fn bytes(path: &Path) -> Vec<u8> {
    fs::read(path).unwrap_or_else(|error| quit(&format!("cannot read {}: {error}", path.display())))
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| {
        quit(&format!(
            "cannot read {}: {error} (run: pnpm --dir web build)",
            path.display()
        ))
    })
}

fn quit(message: &str) -> ! {
    panic!("santi-api embed check: {message}");
}
