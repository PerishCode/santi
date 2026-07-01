use async_trait::async_trait;
use futures_util::stream;
use santi_core::{
    CreateSoulRequest, MessagePart, ObjectBucket, ObjectUri, SESSION_WORKSPACE_URI,
    SOUL_WORKSPACE_URI, SantiService, SantiServiceConfig, SendSessionRequest, session_memory_uri,
    soul_memory_uri,
};
use santi_provider::{
    ProviderClient, ProviderEvent, ProviderFunctionCall, ProviderItem, ProviderMetadata,
    ProviderRequest, ProviderStream,
};
use serde_json::json;
use std::{
    path::Path,
    sync::{Arc, Mutex},
};
use tokio::time::{Duration, sleep};

#[derive(Clone, Default)]
struct FakeProvider {
    requests: Arc<Mutex<Vec<ProviderRequest>>>,
    request_tool: bool,
}

#[async_trait]
impl ProviderClient for FakeProvider {
    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata {
            provider: Arc::from("fake-provider"),
            model: "fake-model".to_string(),
        }
    }

    async fn stream_response(&self, request: ProviderRequest) -> Result<ProviderStream, String> {
        let index = {
            let mut requests = self.requests.lock().unwrap();
            requests.push(request);
            requests.len()
        };
        if self.request_tool && index == 1 {
            return Ok(Box::pin(stream::iter(vec![
                Ok(ProviderEvent::FunctionCallRequested(ProviderFunctionCall {
                    response_id: "resp_tool".to_string(),
                    item_id: Some("item_tool".to_string()),
                    item: json!({
                        "type": "function_call",
                        "id": "item_tool",
                        "call_id": "call_shell",
                        "name": "shell",
                        "arguments": r#"{"command":"pwd && printf \"\\n$SANTI_SESSION_MEMORY_DIR\"","cwd":"session://"}"#,
                    }),
                    call_id: "call_shell".to_string(),
                    name: "shell".to_string(),
                    arguments_raw: r#"{"command":"pwd && printf \"\\n$SANTI_SESSION_MEMORY_DIR\"","cwd":"session://"}"#.to_string(),
                    arguments: json!({
                        "command": "pwd && printf \"\\n$SANTI_SESSION_MEMORY_DIR\"",
                        "cwd": SESSION_WORKSPACE_URI
                    }),
                })),
                Ok(ProviderEvent::Completed {
                    provider_response_id: Some("resp_tool".to_string()),
                }),
            ])));
        }
        Ok(Box::pin(stream::iter(vec![
            Ok(ProviderEvent::TextDelta("hi from runtime".to_string())),
            Ok(ProviderEvent::Completed {
                provider_response_id: Some("fake-response-id".to_string()),
            }),
        ])))
    }
}

#[tokio::test]
async fn sends_with_runtime() {
    let temp = tempfile::tempdir().expect("temp dir");
    let provider = Arc::new(FakeProvider::default());
    let service = SantiService::open(
        SantiServiceConfig {
            database_path: temp.path().join("santi.sqlite").display().to_string(),
            runtime_root: temp.path().join("runtime").display().to_string(),
            execution_root: temp.path().join("execution").display().to_string(),
            bind_addr: Some("127.0.0.1:0".to_string()),
        },
        provider.clone(),
    )
    .expect("open service");

    let session = service.create_session().expect("create session").session;
    let response = service
        .send_session(
            &session.session.id,
            SendSessionRequest {
                content: vec![MessagePart::Text {
                    text: "hello provider".to_string(),
                }],
                soul_id: None,
            },
        )
        .await
        .expect("send session");

    assert_eq!(response.user_message.content_text, "hello provider");
    assert_eq!(response.turn.status, santi_core::TurnStatus::Running);
    let runtime = wait_for_completed_turn(&service, &session.session.id, &response.turn.id).await;
    assert!(
        runtime
            .messages
            .iter()
            .any(|message| message.content_text == "hi from runtime")
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
    assert!(instructions.contains("You are a distinct soul running inside this Santi instance."));
    assert!(instructions.contains("[santi-meta]"));
    assert!(instructions.contains("channel: santi"));
    assert!(instructions.contains("soul_name: Liberte"));
    assert!(instructions.contains("[santi-soul]"));
    assert!(instructions.contains("[santi-session]"));
    assert!(instructions.contains(&format!(
        "{} will always be displayed in [santi-soul].",
        soul_memory_uri()
    )));
    assert!(instructions.contains(&format!(
        "{} will always be displayed in [santi-session].",
        session_memory_uri()
    )));
    assert!(instructions.contains(&format!(
        "These files have no internal version history; save backups into {SOUL_WORKSPACE_URI} or {SESSION_WORKSPACE_URI} if needed."
    )));
    assert!(
        instructions
            .contains("<santi-system> blocks describe Santi runtime facts in this session.")
    );
    assert!(instructions.contains(
        "They are part of your context, not user speech or your natural-language reply."
    ));
    assert!(
        instructions
            .contains("Read them as session facts about the workspace, runtime, or provider flow.")
    );
    assert!(instructions.contains(&format!("source: {}", soul_memory_uri())));
    assert!(instructions.contains(&format!("source: {}", session_memory_uri())));
    assert!(!instructions.contains("hint:"));
    assert!(!instructions.contains("@soul"));
    assert!(!instructions.contains("@session"));
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
    assert!(tool_descriptions.contains(&session_memory_uri()));
    assert!(!tool_descriptions.contains("@soul"));
    assert!(!tool_descriptions.contains("@session"));

    let detail = service
        .session(&session.session.id)
        .expect("load detail")
        .expect("session");
    assert_eq!(detail.messages.len(), 2);
    assert_eq!(runtime.turns.len(), 1);
}

#[tokio::test]
async fn dispatches_tools() {
    let temp = tempfile::tempdir().expect("temp dir");
    let provider = Arc::new(FakeProvider {
        request_tool: true,
        ..FakeProvider::default()
    });
    let service = SantiService::open(
        SantiServiceConfig {
            database_path: temp.path().join("santi.sqlite").display().to_string(),
            runtime_root: temp.path().join("runtime").display().to_string(),
            execution_root: temp.path().join("execution").display().to_string(),
            bind_addr: Some("127.0.0.1:0".to_string()),
        },
        provider.clone(),
    )
    .expect("open service");

    let session = service.create_session().expect("create session").session;
    let response = service
        .send_session(
            &session.session.id,
            SendSessionRequest {
                content: vec![MessagePart::Text {
                    text: "run tool".to_string(),
                }],
                soul_id: None,
            },
        )
        .await
        .expect("send session");

    assert_eq!(response.turn.status, santi_core::TurnStatus::Running);
    let runtime = wait_for_completed_turn(&service, &session.session.id, &response.turn.id).await;
    assert!(
        runtime
            .messages
            .iter()
            .any(|message| message.content_text == "hi from runtime")
    );
    assert_eq!(runtime.tool_calls.len(), 1);
    assert_eq!(runtime.tool_calls[0].tool_name, "shell");
    assert_eq!(runtime.tool_results.len(), 1);
    assert!(runtime.tool_results[0].error_text.is_none());
    let output = runtime.tool_results[0]
        .output
        .as_ref()
        .expect("tool output");
    let stdout = output
        .get("stdout")
        .and_then(|value| value.as_str())
        .expect("shell stdout");
    let session_memory_dir = Path::new("runtime")
        .join("sessions")
        .join(&session.session.id)
        .join("memory");
    assert!(stdout.contains(&session_memory_dir.display().to_string()));
    let cwd = output
        .get("cwd")
        .and_then(|value| value.as_str())
        .expect("shell cwd");
    assert!(Path::new(cwd).ends_with(&session_memory_dir));

    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].previous_response_id.is_none());
    // Round 2 re-derives input from the timeline: the prior tool call + result
    // are replayed as items (no function_call_outputs side-channel).
    assert!(
        requests[1]
            .input
            .iter()
            .any(|item| matches!(item, ProviderItem::FunctionCall { .. }))
    );
    assert!(
        requests[1]
            .input
            .iter()
            .any(|item| matches!(item, ProviderItem::FunctionCallOutput { .. }))
    );
}

#[tokio::test]
async fn ingest_external_event_triggers_turn() {
    let temp = tempfile::tempdir().expect("temp dir");
    let provider = Arc::new(FakeProvider::default());
    let service = SantiService::open(
        SantiServiceConfig {
            database_path: temp.path().join("santi.sqlite").display().to_string(),
            runtime_root: temp.path().join("runtime").display().to_string(),
            execution_root: temp.path().join("execution").display().to_string(),
            bind_addr: Some("127.0.0.1:0".to_string()),
        },
        provider.clone(),
    )
    .expect("open service");

    let soul_id = service.list_souls().expect("list souls")[0].soul_id.clone();
    let label = "github:ops:issue:PerishCode/santi#42";
    let session_id = service
        .ingest_external_event(&soul_id, label, "an external request arrived".to_string())
        .expect("ingest event");

    // The webhook event is a REQUEST → it wakes the soul on a label-anchored
    // session. Wait for the system-triggered turn to complete.
    let runtime = wait_for_any_completed_turn(&service, &session_id).await;
    assert!(
        runtime
            .turns
            .iter()
            .any(|turn| turn.trigger_type == santi_core::TurnTriggerType::System)
    );
    assert!(
        runtime
            .messages
            .iter()
            .any(|message| message.content_text == "an external request arrived")
    );
    assert!(
        runtime
            .messages
            .iter()
            .any(|message| message.content_text == "hi from runtime")
    );

    // A second event on the same label coalesces onto the same session, not a new one.
    let session_id_again = service
        .ingest_external_event(&soul_id, label, "a follow-up arrived".to_string())
        .expect("ingest second event");
    assert_eq!(session_id_again, session_id);

    // The normalized text reached the provider as a user-role message.
    let requests = provider.requests.lock().unwrap();
    assert!(requests.iter().any(|request| {
        request.input.iter().any(|item| {
            matches!(
                item,
                ProviderItem::Message { role, content }
                    if role == "user" && content == "an external request arrived"
            )
        })
    }));
}

#[tokio::test]
async fn send_session_addresses_explicit_soul() {
    let temp = tempfile::tempdir().expect("temp dir");
    let provider = Arc::new(FakeProvider::default());
    let service = SantiService::open(
        SantiServiceConfig {
            database_path: temp.path().join("santi.sqlite").display().to_string(),
            runtime_root: temp.path().join("runtime").display().to_string(),
            execution_root: temp.path().join("execution").display().to_string(),
            bind_addr: Some("127.0.0.1:0".to_string()),
        },
        provider.clone(),
    )
    .expect("open service");

    let default_soul = service.list_souls().expect("list souls")[0].soul_id.clone();
    let secretary = service
        .create_soul(CreateSoulRequest {
            soul_name: "Secretary".to_string(),
            nickname: "sec".to_string(),
            desc: None,
        })
        .expect("create soul");
    assert_ne!(secretary.soul_id, default_soul);

    // An explicit soul_id binds this (soul, session) pair to that soul.
    let session = service.create_session().expect("create session").session;
    let response = service
        .send_session(
            &session.session.id,
            SendSessionRequest {
                content: vec![MessagePart::Text {
                    text: "for the secretary".to_string(),
                }],
                soul_id: Some(secretary.soul_id.clone()),
            },
        )
        .await
        .expect("send session");
    assert_eq!(response.soul_profile.soul_id, secretary.soul_id);
    assert_eq!(response.soul_session.soul_id, secretary.soul_id);

    // Absent soul_id keeps the pre-multi-soul path: the runtime's default soul.
    let other = service.create_session().expect("create session").session;
    let default_response = service
        .send_session(
            &other.session.id,
            SendSessionRequest {
                content: vec![MessagePart::Text {
                    text: "for whoever".to_string(),
                }],
                soul_id: None,
            },
        )
        .await
        .expect("send session");
    assert_eq!(default_response.soul_profile.soul_id, default_soul);

    // An unknown soul is rejected cleanly (no orphan soul_session), not a 500.
    let stray = service.create_session().expect("create session").session;
    let error = service
        .send_session(
            &stray.session.id,
            SendSessionRequest {
                content: vec![MessagePart::Text {
                    text: "nobody home".to_string(),
                }],
                soul_id: Some("soul_does_not_exist".to_string()),
            },
        )
        .await
        .expect_err("unknown soul should error");
    assert!(error.contains("unknown soul"), "got: {error}");
}

#[tokio::test]
async fn completed_turn_emits_turn_completed_event() {
    let temp = tempfile::tempdir().expect("temp dir");
    let provider = Arc::new(FakeProvider::default());
    let service = SantiService::open(
        SantiServiceConfig {
            database_path: temp.path().join("santi.sqlite").display().to_string(),
            runtime_root: temp.path().join("runtime").display().to_string(),
            execution_root: temp.path().join("execution").display().to_string(),
            bind_addr: Some("127.0.0.1:0".to_string()),
        },
        provider.clone(),
    )
    .expect("open service");

    // Subscribe before sending so no lifecycle event is missed.
    let mut events = service.subscribe_stream();
    let session = service.create_session().expect("create session").session;
    let response = service
        .send_session(
            &session.session.id,
            SendSessionRequest {
                content: vec![MessagePart::Text {
                    text: "say hi".to_string(),
                }],
                soul_id: None,
            },
        )
        .await
        .expect("send session");

    // The CLI `--watch` idle check relies on a terminal turn event carrying the
    // same turn_id the send landed on. Drain the stream until it arrives.
    let turn_id = response.turn.id.clone();
    let completed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await.expect("stream event").payload {
                santi_core::SantiStreamPayload::TurnCompleted { turn_id } => break turn_id,
                _ => continue,
            }
        }
    })
    .await
    .expect("turn_completed within timeout");
    assert_eq!(completed, turn_id);
}

async fn wait_for_any_completed_turn(
    service: &SantiService,
    session_id: &str,
) -> santi_core::SessionRuntimeSnapshot {
    for _ in 0..50 {
        let runtime = service
            .runtime_snapshot(session_id)
            .expect("runtime snapshot")
            .expect("session runtime");
        if runtime
            .turns
            .iter()
            .any(|turn| turn.status == santi_core::TurnStatus::Completed)
        {
            return runtime;
        }
        sleep(Duration::from_millis(20)).await;
    }
    panic!("no turn completed");
}

async fn wait_for_completed_turn(
    service: &SantiService,
    session_id: &str,
    turn_id: &str,
) -> santi_core::SessionRuntimeSnapshot {
    for _ in 0..50 {
        let runtime = service
            .runtime_snapshot(session_id)
            .expect("runtime snapshot")
            .expect("session runtime");
        if runtime
            .turns
            .iter()
            .any(|turn| turn.id == turn_id && turn.status == santi_core::TurnStatus::Completed)
        {
            return runtime;
        }
        sleep(Duration::from_millis(20)).await;
    }
    panic!("turn did not complete");
}

#[tokio::test]
async fn bucket_objects_are_scoped() {
    let temp = tempfile::tempdir().expect("temp dir");
    let service = SantiService::open(
        SantiServiceConfig {
            database_path: temp.path().join("santi.sqlite").display().to_string(),
            runtime_root: temp.path().join("runtime").display().to_string(),
            execution_root: temp.path().join("execution").display().to_string(),
            bind_addr: Some("127.0.0.1:0".to_string()),
        },
        Arc::new(FakeProvider::default()),
    )
    .expect("open service");
    let session = service.create_session().expect("create session").session;
    let bucket = ObjectBucket::new("soul_default", session.session.id.as_str()).expect("bucket");
    let uri = ObjectUri::new(bucket.clone(), "avatars/santi.svg").expect("uri");

    let meta = service
        .put_bucket_object(&uri, b"<svg>avatar</svg>")
        .expect("put object");
    assert_eq!(meta.uri.as_santi_uri(), uri.as_santi_uri());
    assert_eq!(meta.len, 17);
    assert_eq!(
        service
            .renderable_ref(&uri.as_santi_uri())
            .expect("renderable ref"),
        format!(
            "/api/v1/bucket/soul_default/{}/avatars/santi.svg",
            session.session.id
        )
    );

    let object = service
        .get_bucket_object("soul_default", &session.session.id, "avatars/santi.svg")
        .expect("get object")
        .expect("object exists");
    assert_eq!(object.bytes, b"<svg>avatar</svg>");
    let objects = service
        .list_bucket_objects(&bucket, Some("avatars"))
        .expect("list objects");
    assert_eq!(objects.len(), 1);
    assert_eq!(objects[0].uri.key, "avatars/santi.svg");
    let objects = service
        .list_bucket_objects(&bucket, Some("avatars/santi"))
        .expect("list object prefix");
    assert_eq!(objects.len(), 1);
    assert!(service.delete_bucket_object(&uri).expect("delete object"));
    assert!(
        service
            .get_bucket_object("soul_default", &session.session.id, "avatars/santi.svg")
            .expect("get deleted object")
            .is_none()
    );
}

#[tokio::test]
async fn bucket_rejects_unsafe_keys() {
    let temp = tempfile::tempdir().expect("temp dir");
    let service = SantiService::open(
        SantiServiceConfig {
            database_path: temp.path().join("santi.sqlite").display().to_string(),
            runtime_root: temp.path().join("runtime").display().to_string(),
            execution_root: temp.path().join("execution").display().to_string(),
            bind_addr: Some("127.0.0.1:0".to_string()),
        },
        Arc::new(FakeProvider::default()),
    )
    .expect("open service");
    let session = service.create_session().expect("create session").session;

    assert!(
        service
            .get_bucket_object("soul_default", &session.session.id, "../escape.txt")
            .expect_err("unsafe key")
            .contains("object key")
    );
    assert!(
        service
            .get_bucket_object("soul_default", &session.session.id, "bad//key.txt")
            .expect_err("empty segment")
            .contains("object key")
    );
    assert!(
        service
            .get_bucket_object("unknown_soul", &session.session.id, "safe.txt")
            .expect_err("unknown soul")
            .contains("soul not found")
    );
}
