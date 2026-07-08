use async_trait::async_trait;
use futures_util::stream;
#[cfg(unix)]
use rusqlite::{Connection, params};
#[cfg(unix)]
use santi_core::{ActorType, MessageState};
use santi_core::{
    CreateSoulRequest, MaterialKind, MaterialRequest, MessageContent, MessageIntake, MessageKind,
    MessagePart, ObjectBucket, ObjectUri, SOUL_WORKSPACE_URI, STRAND_WORKSPACE_URI, SantiService,
    SantiServiceConfig, SantiStore, SendStrandRequest, soul_memory_uri, strand_memory_uri,
};
use santi_provider::{
    ProviderClient, ProviderEvent, ProviderFunctionCall, ProviderItem, ProviderMetadata,
    ProviderRequest, ProviderStream,
};
use serde_json::json;
use std::{
    fs,
    path::Path,
    sync::{Arc, Mutex},
};
use tokio::{
    sync::Notify,
    time::{Duration, sleep},
};

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
            let command = probe_command();
            let arguments = json!({
                "command": command,
                "cwd": STRAND_WORKSPACE_URI
            });
            let arguments_raw = arguments.to_string();
            return Ok(Box::pin(stream::iter(vec![
                Ok(ProviderEvent::FunctionCallRequested(ProviderFunctionCall {
                    response_id: "resp_tool".to_string(),
                    item_id: Some("item_tool".to_string()),
                    item: json!({
                        "type": "function_call",
                        "id": "item_tool",
                        "call_id": "call_shell",
                        "name": "shell",
                        "arguments": arguments_raw,
                    }),
                    call_id: "call_shell".to_string(),
                    name: "shell".to_string(),
                    arguments_raw,
                    arguments,
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

#[derive(Clone)]
struct GatedFirstProvider {
    requests: Arc<Mutex<Vec<ProviderRequest>>>,
    first_request_seen: Arc<Notify>,
    release_first_request: Arc<Notify>,
}

impl GatedFirstProvider {
    fn new() -> Self {
        Self {
            requests: Arc::new(Mutex::new(Vec::new())),
            first_request_seen: Arc::new(Notify::new()),
            release_first_request: Arc::new(Notify::new()),
        }
    }

    async fn wait_for_first_request(&self) {
        tokio::time::timeout(Duration::from_secs(5), self.first_request_seen.notified())
            .await
            .expect("first provider request observed");
    }

    fn release_first_request(&self) {
        self.release_first_request.notify_one();
    }
}

#[async_trait]
impl ProviderClient for GatedFirstProvider {
    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata {
            provider: Arc::from("gated-provider"),
            model: "gated-model".to_string(),
        }
    }

    async fn stream_response(&self, request: ProviderRequest) -> Result<ProviderStream, String> {
        let index = {
            let mut requests = self.requests.lock().unwrap();
            requests.push(request);
            requests.len()
        };
        if index == 1 {
            self.first_request_seen.notify_one();
            self.release_first_request.notified().await;
        }
        Ok(Box::pin(stream::iter(vec![
            Ok(ProviderEvent::TextDelta(format!(
                "provider response {index}"
            ))),
            Ok(ProviderEvent::Completed {
                provider_response_id: Some(format!("gated-response-{index}")),
            }),
        ])))
    }
}

fn probe_command() -> &'static str {
    if cfg!(windows) {
        "[Console]::Out.WriteLine((Get-Location).Path); [Console]::Out.WriteLine($env:SANTI_STRAND_MEMORY_DIR); [Console]::Out.WriteLine($env:SANTI_SOUL_ID); [Console]::Out.WriteLine($env:SANTI_STRAND_ID)"
    } else {
        "pwd && printf \"\\n$SANTI_STRAND_MEMORY_DIR\\n$SANTI_SOUL_ID\\n$SANTI_STRAND_ID\""
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

    let strand = service.create_strand().expect("create strand").strand;
    let response = service
        .send_strand(
            &strand.id,
            SendStrandRequest {
                content: vec![MessagePart::Text {
                    text: "hello provider".to_string(),
                }],
            },
        )
        .await
        .expect("send strand");

    assert_eq!(
        response
            .user_message
            .expect("driven synchronously")
            .content_text,
        "hello provider"
    );
    assert_eq!(response.turn.status, santi_core::TurnStatus::Running);
    let runtime = wait_for_completed_turn(&service, &strand.id, &response.turn.id).await;
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
    // The [santi] constitution replaces the old preamble prose (encoded default,
    // since no constitution.md is written in this temp runtime).
    assert!(instructions.contains("[santi]"));
    assert!(instructions.contains(
        "santi is an agent runtime: a container that keeps souls and runs their strands."
    ));
    assert!(instructions.contains("[santi-meta]"));
    assert!(instructions.contains("soul_id: soul_default"));
    assert!(instructions.contains("strand_id: "));
    // [santi-meta] is slim: no channel, no soul_name (identity is memory).
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

    let strand = service.create_strand().expect("create strand").strand;
    let response = service
        .send_strand(
            &strand.id,
            SendStrandRequest {
                content: vec![MessagePart::Text {
                    text: "run tool".to_string(),
                }],
            },
        )
        .await
        .expect("send strand");

    assert_eq!(response.turn.status, santi_core::TurnStatus::Running);
    let runtime = wait_for_completed_turn(&service, &strand.id, &response.turn.id).await;
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
    let strand_memory_dir = Path::new("runtime")
        .join("strands")
        .join(&strand.id)
        .join("memory");
    assert!(stdout.contains(&strand_memory_dir.display().to_string()));
    // Self-involved env: the soul's shell inherits its own soul_id + strand_id,
    // so `santi …` from the shell auto-scopes to itself.
    assert!(
        stdout.contains("soul_default"),
        "SANTI_SOUL_ID in shell env: {stdout}"
    );
    assert!(
        stdout.contains(&strand.id),
        "SANTI_STRAND_ID in shell env: {stdout}"
    );
    let cwd = output
        .get("cwd")
        .and_then(|value| value.as_str())
        .expect("shell cwd");
    assert!(Path::new(cwd).ends_with(&strand_memory_dir));

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

    let soul_id = service.list_souls().expect("list souls")[0].id.clone();
    let label = "github:ops:issue:PerishCode/santi#42";
    let santi_core::IngestOutcome::Accepted { strand_id } = service
        .ingest_external_event(&soul_id, label, "an external request arrived".to_string())
        .expect("ingest event")
    else {
        panic!("expected accepted");
    };

    // The webhook event is a REQUEST → it wakes the soul on a label-anchored
    // strand. Wait for the system-triggered turn to complete.
    let runtime = wait_for_any_completed_turn(&service, &strand_id).await;
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

    // A second event on the same label coalesces onto the same strand, not a new one.
    let santi_core::IngestOutcome::Accepted {
        strand_id: strand_id_again,
    } = service
        .ingest_external_event(&soul_id, label, "a follow-up arrived".to_string())
        .expect("ingest second event")
    else {
        panic!("expected accepted");
    };
    assert_eq!(strand_id_again, strand_id);

    // A doorbell is a runtime-authored santi_system fact, not user speech — it
    // reaches the provider as a system-role message (see message_to_provider_item).
    let requests = provider.requests.lock().unwrap();
    assert!(requests.iter().any(|request| {
        request.input.iter().any(|item| {
            matches!(
                item,
                ProviderItem::Message { role, content }
                    if role == "system" && content == "an external request arrived"
            )
        })
    }));
}

/// Boot recovery drains the inbox: content that an adaptor durably enqueued
/// but that never got drained before a crash (nobody called `ingest`'s poke —
/// simulated here by writing straight to the store, bypassing the service)
/// still drives a turn once a fresh service opens against the same db and
/// calls `resume_pending`.
#[tokio::test]
async fn boot_recovery_drains_stranded_inbox_entries() {
    let temp = tempfile::tempdir().expect("temp dir");
    let config = SantiServiceConfig {
        database_path: temp.path().join("santi.sqlite").display().to_string(),
        runtime_root: temp.path().join("runtime").display().to_string(),
        execution_root: temp.path().join("execution").display().to_string(),
        bind_addr: Some("127.0.0.1:0".to_string()),
    };
    let provider = Arc::new(FakeProvider::default());

    let strand_id = {
        let service = SantiService::open(config.clone(), provider.clone()).expect("open service");
        service.create_strand().expect("create strand").strand.id
    };

    // Simulate an adaptor that enqueued content and then the process crashed
    // before any poke ever drained it: write directly to the inbox, bypassing
    // SantiService::ingest/send_strand entirely.
    let store = SantiStore::open(&config.database_path).expect("open store directly");
    store
        .enqueue_inbox(
            &strand_id,
            MessageKind::Text,
            MessageContent::text("stranded before the crash"),
        )
        .expect("enqueue inbox");
    drop(store);

    // A fresh service against the SAME db, as after a restart.
    let service = SantiService::open(config, provider.clone()).expect("reopen service");
    service.resume_pending();

    let runtime = wait_for_any_completed_turn(&service, &strand_id).await;
    assert!(
        runtime
            .messages
            .iter()
            .any(|message| message.content_text == "stranded before the crash")
    );
    assert!(
        runtime
            .messages
            .iter()
            .any(|message| message.content_text == "hi from runtime")
    );
}

/// Graceful shutdown pauses inbox CONSUMPTION (no new turns start) while ingest
/// keeps PRODUCING durably; a later fresh boot then drains what queued up. This
/// is the enabling behavior for self-upgrade: quiesce → stop → swap → start →
/// boot recovery wakes the soul on whatever queued during the window (PHASE-07).
#[tokio::test]
async fn graceful_shutdown_pauses_consumption_but_not_production() {
    let temp = tempfile::tempdir().expect("temp dir");
    let config = SantiServiceConfig {
        database_path: temp.path().join("santi.sqlite").display().to_string(),
        runtime_root: temp.path().join("runtime").display().to_string(),
        execution_root: temp.path().join("execution").display().to_string(),
        bind_addr: Some("127.0.0.1:0".to_string()),
    };
    let provider = Arc::new(FakeProvider::default());

    // A quiescing service: it accepts (durably enqueues) but starts NO turn.
    let strand_id = {
        let service = SantiService::open(config.clone(), provider.clone()).expect("open service");
        service.begin_shutdown();
        assert!(service.is_shutting_down());
        let outcome = service
            .ingest_external_event(
                "soul_default",
                "shutdown:quiesce",
                "arrived while quiescing".to_string(),
            )
            .expect("ingest during shutdown");
        match outcome {
            santi_core::IngestOutcome::Accepted { strand_id } => strand_id,
            other => panic!("expected accepted, got {other:?}"),
        }
    };

    // Consumption paused: no turn was started. Production intact: the record is
    // durably queued (exactly what boot recovery scans for).
    let store = SantiStore::open(&config.database_path).expect("open store directly");
    assert_eq!(
        store.running_turn_count().expect("count"),
        0,
        "shutdown must not start a turn"
    );
    assert!(
        store
            .strands_with_pending_requests()
            .expect("pending")
            .contains(&strand_id),
        "the ingested record must still be durably queued"
    );
    drop(store);

    // A fresh service (not shutting down) drains the backlog on boot.
    let service = SantiService::open(config, provider.clone()).expect("reopen service");
    service.resume_pending();
    let runtime = wait_for_any_completed_turn(&service, &strand_id).await;
    assert!(
        runtime
            .messages
            .iter()
            .any(|message| message.content_text == "arrived while quiescing")
    );
}

#[tokio::test]
async fn send_strand_targets_the_strands_own_soul() {
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

    let default_soul = service.list_souls().expect("list souls")[0].id.clone();
    let secretary = service
        .create_soul(CreateSoulRequest {
            memory: Some("# I am the secretary".to_string()),
        })
        .expect("create soul");
    assert_ne!(secretary.id, default_soul);

    // `create_strand` (client-facing, no label) always binds the default soul —
    // multi-soul-per-strand is gone, so a non-default soul is reached only via a
    // label-anchored strand (e.g. ingest_external_event), not by picking a soul
    // at send time.
    let strand = service.create_strand().expect("create strand").strand;
    assert_eq!(strand.soul_id, default_soul);
    let response = service
        .send_strand(
            &strand.id,
            SendStrandRequest {
                content: vec![MessagePart::Text {
                    text: "for whoever".to_string(),
                }],
            },
        )
        .await
        .expect("send strand");
    assert_eq!(response.strand.soul_id, default_soul);

    // A label-anchored strand can be owned by a non-default soul (via ingest).
    let santi_core::IngestOutcome::Accepted {
        strand_id: secretary_strand_id,
    } = service
        .ingest_external_event(
            &secretary.id,
            "github:issue:1",
            "hello secretary".to_string(),
        )
        .expect("ingest event")
    else {
        panic!("expected accepted");
    };
    let secretary_response = service
        .send_strand(
            &secretary_strand_id,
            SendStrandRequest {
                content: vec![MessagePart::Text {
                    text: "for the secretary".to_string(),
                }],
            },
        )
        .await
        .expect("send strand");
    assert_eq!(secretary_response.strand.soul_id, secretary.id);

    // An unknown strand id is rejected cleanly, not a 500.
    let error = service
        .send_strand(
            "ss_does_not_exist",
            SendStrandRequest {
                content: vec![MessagePart::Text {
                    text: "nobody home".to_string(),
                }],
            },
        )
        .await
        .expect_err("unknown strand should error");
    assert!(error.contains("strand not found"), "got: {error}");
}

#[tokio::test]
async fn compact_reminder_after_completed_turn_does_not_drive_duplicate_turn() {
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

    let strand = service.create_strand().expect("create strand").strand;
    let response = service
        .send_strand(
            &strand.id,
            SendStrandRequest {
                content: vec![MessagePart::Text {
                    text: "x".repeat(128 * 1024),
                }],
            },
        )
        .await
        .expect("send strand");

    let _runtime = wait_for_completed_turn(&service, &strand.id, &response.turn.id).await;

    // The provider-input observation is drained only after completion. Wait for
    // the compact reminder Record to materialize, then give the completion
    // re-poke a chance to run. That re-poke must not start a second turn: the
    // reminder is a Record, not a new inbox Request.
    let _runtime =
        wait_for_message_containing(&service, &strand.id, "kind: compact_reminder").await;
    sleep(Duration::from_millis(100)).await;
    let runtime = service
        .runtime_snapshot(&strand.id)
        .expect("runtime snapshot")
        .expect("strand runtime");

    let requests = provider.requests.lock().unwrap();
    assert_eq!(
        requests.len(),
        1,
        "compact reminder completion path must not call the provider again"
    );
    assert_eq!(
        runtime.turns.len(),
        1,
        "compact reminder completion path must not create any duplicate turn row"
    );
    assert_eq!(
        runtime
            .turns
            .iter()
            .filter(|turn| turn.status == santi_core::TurnStatus::Completed)
            .count(),
        1,
        "compact reminder completion path must leave exactly one completed turn"
    );
    assert_eq!(
        runtime
            .messages
            .iter()
            .filter(|message| message.content_text.contains("kind: compact_reminder"))
            .count(),
        1,
        "large input should materialize exactly one compact reminder Record"
    );
}

#[tokio::test]
async fn request_arriving_during_running_turn_drives_one_follow_on_turn() {
    let temp = tempfile::tempdir().expect("temp dir");
    let provider = Arc::new(GatedFirstProvider::new());
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

    let strand = service.create_strand().expect("create strand").strand;
    let first = service
        .send_strand(
            &strand.id,
            SendStrandRequest {
                content: vec![MessagePart::Text {
                    text: "first request".to_string(),
                }],
            },
        )
        .await
        .expect("send first request");
    let first_turn_id = first.turn.id.clone();
    assert_eq!(
        first
            .user_message
            .as_ref()
            .expect("first send drove synchronously")
            .content_text,
        "first request"
    );

    provider.wait_for_first_request().await;

    let second = service
        .send_strand(
            &strand.id,
            SendStrandRequest {
                content: vec![MessagePart::Text {
                    text: "second request".to_string(),
                }],
            },
        )
        .await
        .expect("send second request while first is running");
    assert_eq!(
        second.turn.id, first_turn_id,
        "a send during a running turn should report the turn it coalesced into"
    );
    assert!(
        second.user_message.is_none(),
        "coalesced send is still in the inbox, not yet a timeline message"
    );

    let running = service
        .runtime_snapshot(&strand.id)
        .expect("runtime snapshot")
        .expect("strand runtime");
    assert_eq!(running.turns.len(), 1);
    assert_eq!(running.turns[0].status, santi_core::TurnStatus::Running);
    assert_eq!(count_messages(&running, "first request"), 1);
    assert_eq!(count_messages(&running, "second request"), 0);
    assert_eq!(provider.requests.lock().unwrap().len(), 1);

    provider.release_first_request();
    let runtime = wait_for_completed_turn_count(&service, &strand.id, 2).await;
    let requests = provider.requests.lock().unwrap();
    assert_eq!(
        requests.len(),
        2,
        "one queued real request should drive exactly one follow-on provider call"
    );
    assert!(
        provider_messages(&requests[0]).contains(&("user", "first request")),
        "first provider call should contain the original request"
    );
    assert!(
        !provider_messages(&requests[0]).contains(&("user", "second request")),
        "coalesced request must not leak into the already-built first provider input"
    );
    let second_input = provider_messages(&requests[1]);
    assert!(
        second_input.contains(&("user", "first request")),
        "follow-on provider call should replay the prior request"
    );
    assert!(
        second_input.contains(&("assistant", "provider response 1")),
        "follow-on provider call should replay the first assistant response"
    );
    assert!(
        second_input.contains(&("user", "second request")),
        "follow-on provider call should include the coalesced real request"
    );
    drop(requests);

    assert_eq!(runtime.turns.len(), 2);
    assert!(
        runtime
            .turns
            .iter()
            .all(|turn| turn.status == santi_core::TurnStatus::Completed)
    );
    assert_eq!(count_messages(&runtime, "first request"), 1);
    assert_eq!(count_messages(&runtime, "second request"), 1);
    assert_eq!(count_messages(&runtime, "provider response 1"), 1);
    assert_eq!(count_messages(&runtime, "provider response 2"), 1);
}

#[tokio::test]
async fn coalesced_request_drain_provenance_preserves_original_enqueue_time() {
    let temp = tempfile::tempdir().expect("temp dir");
    let provider = Arc::new(GatedFirstProvider::new());
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

    let strand = service.create_strand().expect("create strand").strand;
    let first = service
        .send_strand(
            &strand.id,
            SendStrandRequest {
                content: vec![MessagePart::Text {
                    text: "first request".to_string(),
                }],
            },
        )
        .await
        .expect("send first request");
    let first_message_id = first
        .user_message
        .as_ref()
        .expect("first send drains immediately")
        .message
        .id
        .clone();

    provider.wait_for_first_request().await;

    let second = service
        .send_strand(
            &strand.id,
            SendStrandRequest {
                content: vec![MessagePart::Text {
                    text: "second request".to_string(),
                }],
            },
        )
        .await
        .expect("send second request while first runs");
    assert!(second.user_message.is_none());

    provider.release_first_request();
    let runtime = wait_for_completed_turn_count(&service, &strand.id, 2).await;

    let second_message = runtime
        .messages
        .iter()
        .find(|message| message.content_text == "second request")
        .expect("second message drained after first turn");
    let second_event = runtime
        .message_events
        .iter()
        .find(|event| event.message_id == second_message.message.id)
        .expect("second message drain event");

    assert_eq!(second_event.payload["kind"], "inbox_drain");
    assert_eq!(
        second_event.payload["message_id"],
        second_message.message.id
    );
    assert_eq!(
        second_event.payload["drained_at"],
        second_message.message.created_at
    );
    assert_eq!(second_event.created_at, second_message.message.created_at);
    assert_eq!(
        second_event.payload["source"]["type"], "strand_send",
        "direct sends should carry caller/source shape"
    );
    assert_eq!(second_event.payload["source"]["ref"], strand.id);

    let follow_on_turn = runtime
        .turns
        .iter()
        .find(|turn| turn.id != first.turn.id)
        .expect("follow-on turn");
    assert_eq!(
        second_event.payload["committing_turn_id"], follow_on_turn.id,
        "the drain event should name the turn that committed the pending request"
    );

    let enqueued_at = second_event.payload["enqueued_at"]
        .as_str()
        .expect("enqueued_at string");
    assert!(
        enqueued_at <= second_message.message.created_at.as_str(),
        "enqueue time should not be later than drain/message time"
    );

    let first_event = runtime
        .message_events
        .iter()
        .find(|event| event.message_id == first_message_id)
        .expect("first message drain event");
    assert_ne!(
        first_event.payload["inbox_id"], second_event.payload["inbox_id"],
        "each inbound request should keep its own inbox id provenance"
    );
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
    let strand = service.create_strand().expect("create strand").strand;
    let response = service
        .send_strand(
            &strand.id,
            SendStrandRequest {
                content: vec![MessagePart::Text {
                    text: "say hi".to_string(),
                }],
            },
        )
        .await
        .expect("send strand");

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
    strand_id: &str,
) -> santi_core::StrandRuntimeSnapshot {
    for _ in 0..50 {
        let runtime = service
            .runtime_snapshot(strand_id)
            .expect("runtime snapshot")
            .expect("strand runtime");
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

async fn wait_for_completed_turn_count(
    service: &SantiService,
    strand_id: &str,
    count: usize,
) -> santi_core::StrandRuntimeSnapshot {
    for _ in 0..50 {
        let runtime = service
            .runtime_snapshot(strand_id)
            .expect("runtime snapshot")
            .expect("strand runtime");
        if runtime
            .turns
            .iter()
            .filter(|turn| turn.status == santi_core::TurnStatus::Completed)
            .count()
            >= count
        {
            return runtime;
        }
        sleep(Duration::from_millis(20)).await;
    }
    panic!("{count} turns did not complete");
}

fn count_messages(runtime: &santi_core::StrandRuntimeSnapshot, text: &str) -> usize {
    runtime
        .messages
        .iter()
        .filter(|message| message.content_text == text)
        .count()
}

fn provider_messages(request: &ProviderRequest) -> Vec<(&str, &str)> {
    request
        .input
        .iter()
        .filter_map(|item| match item {
            ProviderItem::Message { role, content } => Some((role.as_str(), content.as_str())),
            _ => None,
        })
        .collect()
}

async fn wait_for_completed_turn(
    service: &SantiService,
    strand_id: &str,
    turn_id: &str,
) -> santi_core::StrandRuntimeSnapshot {
    for _ in 0..50 {
        let runtime = service
            .runtime_snapshot(strand_id)
            .expect("runtime snapshot")
            .expect("strand runtime");
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

async fn wait_for_message_containing(
    service: &SantiService,
    strand_id: &str,
    needle: &str,
) -> santi_core::StrandRuntimeSnapshot {
    for _ in 0..50 {
        let runtime = service
            .runtime_snapshot(strand_id)
            .expect("runtime snapshot")
            .expect("strand runtime");
        if runtime
            .messages
            .iter()
            .any(|message| message.content_text.contains(needle))
        {
            return runtime;
        }
        sleep(Duration::from_millis(20)).await;
    }
    panic!("message containing {needle:?} did not appear");
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
    let strand = service.create_strand().expect("create strand").strand;
    let bucket = ObjectBucket::new("soul_default", strand.id.as_str()).expect("bucket");
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
            strand.id
        )
    );

    let object = service
        .get_bucket_object("soul_default", &strand.id, "avatars/santi.svg")
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
            .get_bucket_object("soul_default", &strand.id, "avatars/santi.svg")
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
    let strand = service.create_strand().expect("create strand").strand;

    assert!(
        service
            .get_bucket_object("soul_default", &strand.id, "../escape.txt")
            .expect_err("unsafe key")
            .contains("object key")
    );
    assert!(
        service
            .get_bucket_object("soul_default", &strand.id, "bad//key.txt")
            .expect_err("empty segment")
            .contains("object key")
    );
    assert!(
        service
            .get_bucket_object("unknown_soul", &strand.id, "safe.txt")
            .expect_err("unknown soul")
            .contains("soul not found")
    );
}

#[test]
fn fork_strand_syncs_workspace_snapshot() {
    let temp = tempfile::tempdir().expect("temp dir");
    let runtime_root = temp.path().join("runtime");
    let service = SantiService::open(
        SantiServiceConfig {
            database_path: temp.path().join("santi.sqlite").display().to_string(),
            runtime_root: runtime_root.display().to_string(),
            execution_root: temp.path().join("execution").display().to_string(),
            bind_addr: Some("127.0.0.1:0".to_string()),
        },
        Arc::new(FakeProvider::default()),
    )
    .expect("open service");
    let parent = service.create_strand().expect("create parent").strand;
    let parent_dir = runtime_root.join("strands").join(&parent.id).join("memory");
    fs::create_dir_all(parent_dir.join("notes")).expect("create parent workspace");
    fs::write(parent_dir.join("MEMORY.md"), "# Parent strand memory\n").expect("write memory");
    fs::write(parent_dir.join("notes/plan.md"), "plan v1\n").expect("write note");

    let child = service.fork_strand(&parent.id).expect("fork").strand;
    let child_dir = runtime_root.join("strands").join(&child.id).join("memory");
    assert_eq!(
        fs::read_to_string(child_dir.join("MEMORY.md")).expect("child memory"),
        "# Parent strand memory\n"
    );
    assert_eq!(
        fs::read_to_string(child_dir.join("notes/plan.md")).expect("child note"),
        "plan v1\n"
    );

    fs::write(parent_dir.join("notes/plan.md"), "parent changed\n").expect("rewrite parent");
    assert_eq!(
        fs::read_to_string(child_dir.join("notes/plan.md")).expect("child note unchanged"),
        "plan v1\n"
    );
}

#[cfg(unix)]
#[test]
fn fork_strand_rejects_symlink_workspace_and_rolls_back_child() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db = temp.path().join("santi.sqlite");
    let runtime_root = temp.path().join("runtime");
    let service = SantiService::open(
        SantiServiceConfig {
            database_path: db.display().to_string(),
            runtime_root: runtime_root.display().to_string(),
            execution_root: temp.path().join("execution").display().to_string(),
            bind_addr: Some("127.0.0.1:0".to_string()),
        },
        Arc::new(FakeProvider::default()),
    )
    .expect("open service");
    let parent = service.create_strand().expect("create parent").strand;
    {
        let store = SantiStore::open(&db).expect("open store directly");
        let first = store
            .append_message(
                &parent.id,
                ActorType::System,
                store.system_actor_id(),
                MessageContent::text("first"),
                MessageState::Fixed,
                MessageIntake::Record,
            )
            .expect("append first")
            .strand_message;
        let second = store
            .append_message(
                &parent.id,
                ActorType::System,
                store.system_actor_id(),
                MessageContent::text("second"),
                MessageState::Fixed,
                MessageIntake::Record,
            )
            .expect("append second")
            .strand_message;
        store
            .create_compact(
                &parent.id,
                &first.message.id,
                &second.message.id,
                "parent compact",
            )
            .expect("parent compact");
    }
    let parent_dir = runtime_root.join("strands").join(&parent.id).join("memory");
    fs::create_dir_all(&parent_dir).expect("create parent workspace");
    fs::write(parent_dir.join("real.md"), "real\n").expect("write target");
    std::os::unix::fs::symlink("real.md", parent_dir.join("link.md")).expect("create symlink");

    let err = service
        .fork_strand(&parent.id)
        .expect_err("symlink workspace should fail fork");
    assert!(
        err.contains("cannot copy symlink"),
        "unexpected error: {err}"
    );

    let strands = service.list_strands().expect("list strands");
    assert_eq!(strands.len(), 1, "fork child strand should be rolled back");
    assert_eq!(strands[0].id, parent.id);
    let runtime = runtime_root.join("strands");
    let child_dirs = fs::read_dir(&runtime)
        .expect("runtime strands dir")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name() != parent.id.as_str())
        .collect::<Vec<_>>();
    assert!(
        child_dirs.is_empty(),
        "failed fork should not leave child workspace dirs: {child_dirs:?}"
    );

    let conn = Connection::open(&db).expect("open sqlite");
    let child_strands: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM strands WHERE parent_strand_id = ?1",
            params![parent.id],
            |row| row.get(0),
        )
        .expect("count child strands");
    assert_eq!(child_strands, 0, "child strand row should be rolled back");
    let child_entries: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM r_strand_entries WHERE strand_id != ?1",
            params![parent.id],
            |row| row.get(0),
        )
        .expect("count child entries");
    assert_eq!(
        child_entries, 0,
        "child r_strand_entries should be rolled back"
    );
    let child_compacts: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM compacts WHERE strand_id != ?1",
            params![parent.id],
            |row| row.get(0),
        )
        .expect("count child compacts");
    assert_eq!(child_compacts, 0, "child compacts should be rolled back");
}

#[test]
fn fork_strand_system_prompt_renders_topology_only() {
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
    let parent = service.create_strand().expect("create parent").strand;
    let store = SantiStore::open(temp.path().join("santi.sqlite")).expect("open store directly");
    store
        .append_santi_system_message(
            &parent.id,
            MessageContent::text("<system_message>\nkind: seed\n</system_message>"),
            MessageIntake::Record,
        )
        .expect("append parent entry");
    drop(store);
    let child = service.fork_strand(&parent.id).expect("fork").strand;

    let text = service
        .strand_material(
            &child.id,
            MaterialRequest {
                kind: MaterialKind::SystemPrompt,
            },
        )
        .expect("system prompt")
        .text;

    assert!(text.contains("[santi-fork]"));
    assert!(text.contains(&format!("parent_strand_id: {}", parent.id)));
    assert!(text.contains("fork_point: 1"));
    assert!(!text.contains("merge"));
    assert!(!text.contains("sandbox"));
    assert!(!text.contains("recommend"));
}
