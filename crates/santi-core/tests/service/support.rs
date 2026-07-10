pub(crate) use async_trait::async_trait;
pub(crate) use futures_util::stream;
#[cfg(unix)]
pub(crate) use rusqlite::{Connection, params};
pub(crate) use santi_core::{
    ActorType, CreateSoulRequest, InboxSource, MaterialKind, MaterialRequest, MessageContent,
    MessageIntake, MessageKind, MessagePart, MessageState, ObjectBucket, ObjectUri,
    SOUL_WORKSPACE_URI, STRAND_WORKSPACE_URI, SantiService, SantiServiceConfig, SantiStore,
    SendStrandRequest, ToolCallProvenance, soul_memory_uri, strand_memory_uri,
};
pub(crate) use santi_provider::{
    ProviderClient, ProviderContextBudget, ProviderEvent, ProviderFunctionCall, ProviderItem,
    ProviderMetadata, ProviderRequest, ProviderStream,
};
pub(crate) use serde_json::json;
pub(crate) use std::{
    fs,
    path::Path,
    sync::{Arc, Mutex},
};
pub(crate) use tokio::{
    sync::Notify,
    time::{Duration, sleep},
};

#[derive(Clone, Default)]
pub(crate) struct FakeProvider {
    pub(crate) requests: Arc<Mutex<Vec<ProviderRequest>>>,
    pub(crate) request_tool: bool,
    pub(crate) input_budget_bytes: Option<usize>,
}

#[async_trait]
impl ProviderClient for FakeProvider {
    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata {
            provider: Arc::from("fake-provider"),
            model: "fake-model".to_string(),
            context_budget: self.input_budget_bytes.map(|input_budget_bytes| {
                ProviderContextBudget {
                    input_budget_bytes,
                    source: "test".to_string(),
                }
            }),
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
pub(crate) struct LargeToolCallProvider {
    pub(crate) requests: Arc<Mutex<Vec<ProviderRequest>>>,
    pub(crate) input_budget_bytes: usize,
}

#[async_trait]
impl ProviderClient for LargeToolCallProvider {
    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata {
            provider: Arc::from("large-tool-provider"),
            model: "large-tool-model".to_string(),
            context_budget: Some(ProviderContextBudget {
                input_budget_bytes: self.input_budget_bytes,
                source: "test".to_string(),
            }),
        }
    }

    async fn stream_response(&self, request: ProviderRequest) -> Result<ProviderStream, String> {
        let index = {
            let mut requests = self.requests.lock().unwrap();
            requests.push(request);
            requests.len()
        };
        if index == 1 {
            let command = if cfg!(windows) {
                "Write-Output ok"
            } else {
                "printf ok"
            };
            let arguments = json!({
                "command": command,
                "cwd": STRAND_WORKSPACE_URI,
                "unused_payload": "x".repeat(200_000),
            });
            let arguments_raw = arguments.to_string();
            return Ok(Box::pin(stream::iter(vec![
                Ok(ProviderEvent::TextDelta(
                    "assistant before tool".to_string(),
                )),
                Ok(ProviderEvent::FunctionCallRequested(ProviderFunctionCall {
                    response_id: "resp_large_tool".to_string(),
                    item_id: Some("item_large_tool".to_string()),
                    item: json!({
                        "type": "function_call",
                        "id": "item_large_tool",
                        "call_id": "call_large_tool",
                        "name": "shell",
                        "arguments": arguments_raw,
                    }),
                    call_id: "call_large_tool".to_string(),
                    name: "shell".to_string(),
                    arguments_raw,
                    arguments,
                })),
                Ok(ProviderEvent::Completed {
                    provider_response_id: Some("resp_large_tool".to_string()),
                }),
            ])));
        }
        Ok(Box::pin(stream::iter(vec![Ok(ProviderEvent::Completed {
            provider_response_id: Some("resp_after_large_tool".to_string()),
        })])))
    }
}

#[derive(Clone)]
pub(crate) struct GatedFirstProvider {
    pub(crate) requests: Arc<Mutex<Vec<ProviderRequest>>>,
    pub(crate) first_request_seen: Arc<Notify>,
    pub(crate) release_first_request: Arc<Notify>,
}

impl GatedFirstProvider {
    pub(crate) fn new() -> Self {
        Self {
            requests: Arc::new(Mutex::new(Vec::new())),
            first_request_seen: Arc::new(Notify::new()),
            release_first_request: Arc::new(Notify::new()),
        }
    }

    pub(crate) async fn wait_for_first_request(&self) {
        tokio::time::timeout(Duration::from_secs(5), self.first_request_seen.notified())
            .await
            .expect("first provider request observed");
    }

    pub(crate) fn release_first_request(&self) {
        self.release_first_request.notify_one();
    }
}

#[async_trait]
impl ProviderClient for GatedFirstProvider {
    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata {
            provider: Arc::from("gated-provider"),
            model: "gated-model".to_string(),
            context_budget: None,
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

pub(crate) async fn wait_any_completed(
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

pub(crate) async fn wait_completed_count(
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

pub(crate) fn count_messages(runtime: &santi_core::StrandRuntimeSnapshot, text: &str) -> usize {
    runtime
        .messages
        .iter()
        .filter(|message| message.content_text == text)
        .count()
}

pub(crate) fn provider_messages(request: &ProviderRequest) -> Vec<(&str, &str)> {
    request
        .input
        .iter()
        .filter_map(|item| match item {
            ProviderItem::Message { role, content } => Some((role.as_str(), content.as_str())),
            _ => None,
        })
        .collect()
}

pub(crate) async fn wait_for_completed_turn(
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

pub(crate) async fn wait_for_failed_turn(
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
            .any(|turn| turn.id == turn_id && turn.status == santi_core::TurnStatus::Failed)
        {
            return runtime;
        }
        sleep(Duration::from_millis(20)).await;
    }
    panic!("turn did not fail");
}

pub(crate) async fn wait_for_message_containing(
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
