use std::{fs, sync::Arc};

use async_trait::async_trait;
use futures_util::stream;
use santi_core::{
    MaterialKind, MaterialRequest, SESSION_WORKSPACE_URI, SOUL_WORKSPACE_URI, SantiService,
    SantiServiceConfig, SessionMaterial, session_memory_uri, soul_memory_uri,
};
use santi_provider::{ProviderClient, ProviderMetadata, ProviderStream};

#[derive(Clone)]
struct FakeProvider;

#[async_trait]
impl ProviderClient for FakeProvider {
    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata {
            provider: Arc::from("fake-provider"),
            model: "fake-model".to_string(),
        }
    }

    async fn stream_response(
        &self,
        _request: santi_provider::ProviderRequest,
    ) -> Result<ProviderStream, String> {
        Ok(Box::pin(stream::empty()))
    }
}

#[test]
fn renders_material_shape() {
    let harness = PromptHarness::open();
    harness.write_soul("---\nplain: value\n---\n# Soul");
    harness.write_session("# Session");

    let text = harness.system_prompt().text;

    assert!(text.contains("You are a distinct soul running inside this Santi instance."));
    assert!(text.contains("[santi-meta]"));
    assert!(text.contains("channel: santi"));
    assert!(text.contains("soul_id: soul_default"));
    // soul_profile dissolved: no name in [santi-meta] (identity is in memory).
    assert!(!text.contains("soul_name"));
    assert!(text.contains(&format!(
        "{} will always be displayed in [santi-soul].",
        soul_memory_uri()
    )));
    assert!(text.contains(&format!(
        "{} will always be displayed in [santi-session].",
        session_memory_uri()
    )));
    assert!(text.contains(&format!(
        "These files have no internal version history; save backups into {SOUL_WORKSPACE_URI} or {SESSION_WORKSPACE_URI} if needed."
    )));
    assert!(text.contains("<santi-system> blocks describe Santi runtime facts in this session."));
    assert!(text.contains(
        "They are part of your context, not user speech or your natural-language reply."
    ));
    assert!(
        text.contains("Read them as session facts about the workspace, runtime, or provider flow.")
    );
    assert!(text.contains("[santi-soul]"));
    assert!(text.contains("[santi-session]"));
    assert!(text.contains(&format!("source: {}", soul_memory_uri())));
    assert!(text.contains(&format!("source: {}", session_memory_uri())));
    assert!(text.contains("content:\n---\nplain: value\n---\n# Soul"));
    assert!(text.contains("content:\n# Session"));
    assert!(!text.contains("hint:"));
    assert!(!text.contains("@soul"));
    assert!(!text.contains("@session"));
}

#[test]
fn leaves_frontmatter_plain() {
    let harness = PromptHarness::open();
    harness.write_soul("---\nplain: value\n---\n# Soul");

    let text = harness.system_prompt().text;

    assert!(text.contains("content:\n---\nplain: value\n---\n# Soul"));
    assert!(!text.contains("hint:"));
}

struct PromptHarness {
    _temp: tempfile::TempDir,
    service: SantiService,
    session_id: String,
    runtime_root: std::path::PathBuf,
}

impl PromptHarness {
    fn open() -> Self {
        let temp = tempfile::tempdir().expect("temp dir");
        let runtime_root = temp.path().join("runtime");
        let service = SantiService::open(
            SantiServiceConfig {
                database_path: temp.path().join("santi.sqlite").display().to_string(),
                runtime_root: runtime_root.display().to_string(),
                execution_root: temp.path().join("execution").display().to_string(),
                bind_addr: Some("127.0.0.1:0".to_string()),
            },
            Arc::new(FakeProvider),
        )
        .expect("open service");
        let session_id = service.create_session().expect("create session").session.id;
        Self {
            _temp: temp,
            service,
            session_id,
            runtime_root,
        }
    }

    fn write_soul(&self, text: &str) {
        // Per-soul home: the default soul's memory lives under souls/<soul_id>.
        let path = self
            .runtime_root
            .join("souls")
            .join("soul_default")
            .join("memory");
        fs::create_dir_all(&path).expect("create soul dir");
        fs::write(path.join("MEMORY.md"), text).expect("write soul");
    }

    fn write_session(&self, text: &str) {
        let path = self
            .runtime_root
            .join("sessions")
            .join(&self.session_id)
            .join("memory");
        fs::create_dir_all(&path).expect("create session dir");
        fs::write(path.join("MEMORY.md"), text).expect("write session");
    }

    fn system_prompt(&self) -> SessionMaterial {
        self.service
            .session_material(
                &self.session_id,
                MaterialRequest {
                    kind: MaterialKind::SystemPrompt,
                },
            )
            .expect("system prompt material")
    }
}
