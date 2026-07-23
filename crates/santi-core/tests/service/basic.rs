use super::support::*;
use santi_core::service::{self, Service};
use santi_core::{message, strand};

#[tokio::test]
async fn sends_with_runtime() {
    let temp = tempfile::tempdir().expect("temp dir");
    let provider = Arc::new(FakeProvider::default());
    let service = Service::open(
        service::Config {
            database_path: temp.path().join("santi.sqlite").display().to_string(),
            runtime_root: temp.path().join("runtime").display().to_string(),
            execution_root: temp.path().join("execution").display().to_string(),
            bind_addr: Some("127.0.0.1:0".to_string()),
            constitution_path: None,
        },
        provider.clone(),
    )
    .expect("open service");

    let strand = service.create_strand().expect("create strand").strand;
    let response = service
        .send_strand(
            &strand.id,
            strand::Post {
                content: vec![message::Part::Text {
                    text: "hello provider".to_string(),
                }],
            },
        )
        .await
        .expect("send strand");

    assert_eq!(
        response
            .message
            .as_ref()
            .expect("driven synchronously")
            .text,
        "hello provider"
    );
    assert_eq!(
        accepted_turn(&response).status,
        santi_core::turn::Status::Running
    );
    let runtime = Probe::new(&service)
        .completed_turn(&strand.id, &accepted_turn(&response).id)
        .await;
    assert!(
        runtime
            .messages
            .iter()
            .any(|message| message.text == "hi from runtime")
    );

    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].model, "fake-model");
    assert_eq!(requests[0].input.len(), 1);
    match &requests[0].input[0] {
        ProviderItem::Message { role, content } => {
            assert_eq!(role, "user");
            assert_eq!(content, "hello provider");
        }
        other => panic!("expected text message, got {other:?}"),
    }
    let instructions = requests[0]
        .instructions
        .as_deref()
        .expect("runtime instructions");
    assert!(instructions.contains("[santi]"));
    assert!(instructions.contains(
        "santi is an agent runtime: a container that keeps souls and runs their strands."
    ));
    assert!(instructions.contains("[santi-meta]"));
    assert!(instructions.contains("soul: soul_default"));
    assert!(instructions.contains("strand: "));
    assert!(!instructions.contains("channel: santi"));
    assert!(!instructions.contains("soul_name"));
    assert!(instructions.contains("[santi-soul]"));
    assert!(instructions.contains("[santi-strand]"));
    assert!(instructions.contains(&format!(
        "{} will always be displayed in [santi-soul].",
        soul_memory_uri()
    )));
    assert!(instructions.contains(&format!(
        "{} will always be displayed in [santi-strand].",
        strand_memory_uri()
    )));
    assert!(instructions.contains(&format!(
        "These files have no internal version history; save backups into {SOUL_WORKSPACE_URI} or {STRAND_WORKSPACE_URI} if needed."
    )));
    assert!(
        instructions
            .contains("<system_message> blocks describe Santi runtime facts in this strand.")
    );
    assert!(instructions.contains(
        "They are part of your context, not user speech or your natural-language reply."
    ));
    assert!(
        instructions
            .contains("Read them as strand facts about the workspace, runtime, or provider flow.")
    );
    assert!(instructions.contains(&format!("source: {}", soul_memory_uri())));
    assert!(instructions.contains(&format!("source: {}", strand_memory_uri())));
    assert!(!instructions.contains("hint:"));
    assert!(!instructions.contains("@soul"));
    assert!(!instructions.contains("@strand"));
    assert!(!instructions.contains("<santi-runtime>"));
    assert!(!instructions.contains("<santi-tools>"));
    let tools = requests[0].tools.as_ref().expect("tools");
    let tool_names = tools
        .iter()
        .map(|tool| match tool {
            santi_provider::ProviderTool::Function(tool) => tool.name.as_str(),
        })
        .collect::<Vec<_>>();
    assert_eq!(tool_names, vec!["shell"]);
    let tool_descriptions = tools
        .iter()
        .map(|tool| match tool {
            santi_provider::ProviderTool::Function(tool) => {
                format!("{} {}", tool.description, tool.parameters)
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(tool_descriptions.contains(&soul_memory_uri()));
    assert!(tool_descriptions.contains(&strand_memory_uri()));
    assert!(!tool_descriptions.contains("@soul"));
    assert!(!tool_descriptions.contains("@strand"));

    let detail = service
        .strand(&strand.id)
        .expect("load detail")
        .expect("strand");
    assert_eq!(detail.messages.len(), 2);
    assert_eq!(runtime.turns.len(), 1);
}
