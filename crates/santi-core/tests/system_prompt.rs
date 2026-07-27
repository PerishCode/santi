use santi_core::service::{self, Service};
use std::{fs, sync::Arc};

use async_trait::async_trait;
use futures_util::stream;
use santi_core::material;
use santi_core::{SOULSPACE, STRANDSPACE, soulward, strandward};
use santi_provider::{Metadata, Provider, Streaming};

#[derive(Clone)]
struct FakeProvider;

#[async_trait]
impl Provider for FakeProvider {
    fn metadata(&self) -> Metadata {
        Metadata {
            provider: Arc::from("fake-provider"),
            model: "fake-model".to_string(),
            budget: None,
        }
    }

    async fn stream(&self, _request: santi_provider::Request) -> Result<Streaming, String> {
        Ok(Box::pin(stream::empty()))
    }
}

#[test]
fn renders() {
    let harness = PromptHarness::open();
    harness.write_soul("---\nplain: value\n---\n# Soul");
    harness.write_strand("# Strand");

    let text = harness.system_prompt().text;

    assert!(text.contains("[santi]"));
    assert!(text.contains(
        "santi is an agent runtime: a container that keeps souls and runs their strands."
    ));
    assert!(text.contains("[santi-meta]"));
    assert!(!text.contains("channel: santi"));
    assert!(text.contains("soul: soul_default"));
    assert!(text.contains("strand: "));
    assert!(!text.contains("soul_name"));
    assert!(text.contains(&format!(
        "{} will always be displayed in [santi-soul].",
        soulward()
    )));
    assert!(text.contains(&format!(
        "{} will always be displayed in [santi-strand].",
        strandward()
    )));
    assert!(text.contains(&format!(
        "These files have no internal version history; save backups into {SOULSPACE} or {STRANDSPACE} if needed."
    )));
    assert!(text.contains("<system_message> blocks describe Santi runtime facts in this strand."));
    assert!(text.contains(
        "They are part of your context, not user speech or your natural-language reply."
    ));
    assert!(
        text.contains("Read them as strand facts about the workspace, runtime, or provider flow.")
    );
    assert!(text.contains("[santi-soul]"));
    assert!(text.contains("[santi-strand]"));
    assert!(text.contains(&format!("source: {}", soulward())));
    assert!(text.contains(&format!("source: {}", strandward())));
    assert!(text.contains("content:\n---\nplain: value\n---\n# Soul"));
    assert!(text.contains("content:\n# Strand"));
    assert!(!text.contains("hint:"));
    assert!(!text.contains("@soul"));
    assert!(!text.contains("@strand"));
}

#[test]
fn leaves() {
    let harness = PromptHarness::open();
    harness.write_soul("---\nplain: value\n---\n# Soul");

    let text = harness.system_prompt().text;

    assert!(text.contains("content:\n---\nplain: value\n---\n# Soul"));
    assert!(!text.contains("hint:"));
}

#[test]
fn overrides() {
    let harness = PromptHarness::open();
    harness.write_constitution("my own physics, hot-edited");

    let text = harness.system_prompt().text;

    assert!(text.contains("[santi]\nmy own physics, hot-edited"));
    assert!(!text.contains("santi is an agent runtime: a container that keeps souls"));
}

#[test]
fn fallback() {
    let harness = PromptHarness::open();

    let text = harness.system_prompt().text;

    assert!(text.contains("Your memory is still empty. You are a soul"));
    assert!(!text.to_lowercase().contains("secretary"));
}

#[test]
fn projects() {
    let harness = PromptHarness::open();
    let memory = format!("# Memory\n{}\nSOURCE_TAIL", "界".repeat(90_000));
    harness.write_soul(&memory);

    let text = harness.system_prompt().text;

    assert!(text.contains("kind: soul_memory_projection"));
    assert!(text.contains("allowance_bytes: 250000"));
    assert!(!text.contains("SOURCE_TAIL"));
    let source = fs::read_to_string(harness.runtime.join("souls/soul_default/memory/MEMORY.md"))
        .expect("read source memory");
    assert_eq!(
        source, memory,
        "projection must never rewrite source memory"
    );
}

#[tokio::test]
async fn excludes() {
    let harness = PromptHarness::open();
    let first = harness
        .service
        .evented(santi_core::GENESIS, "stim:operator", "hello".to_string())
        .expect("first strand");
    let second = harness
        .service
        .evented(
            santi_core::GENESIS,
            "github:ops:issue:PerishCode/santi#1",
            "hello".to_string(),
        )
        .expect("second strand");
    let santi_core::ingest::Outcome::Accepted {
        receipt: first_receipt,
    } = first
    else {
        panic!("first ingest rejected");
    };
    let santi_core::ingest::Outcome::Accepted {
        receipt: second_receipt,
    } = second
    else {
        panic!("second ingest rejected");
    };
    let first_text = harness.system_prompt_for(&first_receipt.strand).text;
    let second_text = harness.system_prompt_for(&second_receipt.strand).text;
    assert!(!first_text.contains("stim:operator"));
    assert!(!second_text.contains("github:ops:issue:PerishCode/santi#1"));
}

struct PromptHarness {
    _temp: tempfile::TempDir,
    service: Service,
    strand: String,
    runtime: std::path::PathBuf,
}

impl PromptHarness {
    fn open() -> Self {
        let temp = tempfile::tempdir().expect("temp dir");
        let runtime = temp.path().join("runtime");
        let service = Service::open(
            service::Config {
                database: temp.path().join("santi.sqlite").display().to_string(),
                runtime: runtime.display().to_string(),
                execution: temp.path().join("execution").display().to_string(),
                bind: Some("127.0.0.1:0".to_string()),
                constitution: None,
            },
            Arc::new(FakeProvider),
        )
        .expect("open service");
        let strand = service.weave().expect("create strand").strand.id;
        Self {
            _temp: temp,
            service,
            strand,
            runtime,
        }
    }

    fn write_soul(&self, text: &str) {
        let path = self
            .runtime
            .join("souls")
            .join("soul_default")
            .join("memory");
        fs::create_dir_all(&path).expect("create soul dir");
        fs::write(path.join("MEMORY.md"), text).expect("write soul");
    }

    fn write_strand(&self, text: &str) {
        let path = self
            .runtime
            .join("strands")
            .join(&self.strand)
            .join("memory");
        fs::create_dir_all(&path).expect("create strand dir");
        fs::write(path.join("MEMORY.md"), text).expect("write strand");
    }

    fn write_constitution(&self, text: &str) {
        fs::create_dir_all(&self.runtime).expect("create runtime dir");
        fs::write(self.runtime.join("constitution.md"), text).expect("write constitution");
    }

    fn system_prompt(&self) -> material::Material {
        self.system_prompt_for(&self.strand)
    }

    fn system_prompt_for(&self, strand: &str) -> material::Material {
        self.service
            .material(
                strand,
                material::Request {
                    kind: material::Kind::SystemPrompt,
                },
            )
            .expect("system prompt material")
    }
}
