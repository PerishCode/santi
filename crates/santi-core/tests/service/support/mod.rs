pub(crate) use async_trait::async_trait;
pub(crate) use futures_util::stream;
#[cfg(unix)]
pub(crate) use rusqlite::{Connection, params};
use santi_core::service::Service;
use santi_core::strand;
pub(crate) use santi_core::{
    Draft, Invocation, SOUL_WORKSPACE_URI, STRAND_WORKSPACE_URI, SantiStore, soul_memory_uri,
    strand_memory_uri,
};

mod probe;
pub(crate) use probe::*;

pub(crate) fn accepted_turn(response: &strand::Posted) -> &santi_core::turn::Turn {
    response.turn.as_ref().expect("send should land on a turn")
}
pub(crate) use santi_provider::{Call, Cap, Event, Item, Metadata, Provider, Request, Streaming};
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
    pub(crate) requests: Arc<Mutex<Vec<Request>>>,
    pub(crate) request_tool: bool,
    pub(crate) bytes: Option<usize>,
    pub(crate) fail_for_requests: Option<usize>,
}

#[async_trait]
impl Provider for FakeProvider {
    fn metadata(&self) -> Metadata {
        Metadata {
            provider: Arc::from("fake-provider"),
            model: "fake-model".to_string(),
            budget: self.bytes.map(|bytes| Cap {
                bytes,
                source: "test".to_string(),
            }),
        }
    }

    async fn stream(&self, request: Request) -> Result<Streaming, String> {
        let index = {
            let mut requests = self.requests.lock().unwrap();
            requests.push(request);
            requests.len()
        };
        if self
            .fail_for_requests
            .is_some_and(|failure_count| index <= failure_count)
        {
            return Err("temporary fake-provider outage".to_string());
        }
        if self.request_tool && index == 1 {
            let command = probe_command();
            let arguments = json!({
                "command": command,
                "cwd": STRAND_WORKSPACE_URI
            });
            let raw = arguments.to_string();
            return Ok(Box::pin(stream::iter(vec![
                Ok(Event::Called(Call {
                    response: "resp_tool".to_string(),
                    mark: Some("item_tool".to_string()),
                    item: json!({
                        "type": "function_call",
                        "id": "item_tool",
                        "call_id": "call_shell",
                        "name": "shell",
                        "arguments": raw,
                    }),
                    call: "call_shell".to_string(),
                    name: "shell".to_string(),
                    raw,
                    arguments,
                })),
                Ok(Event::Completed {
                    response: Some("resp_tool".to_string()),
                }),
            ])));
        }
        Ok(Box::pin(stream::iter(vec![
            Ok(Event::Text("hi from runtime".to_string())),
            Ok(Event::Completed {
                response: Some("fake-response-id".to_string()),
            }),
        ])))
    }
}

#[derive(Clone)]
pub(crate) struct LargeToolCallProvider {
    pub(crate) requests: Arc<Mutex<Vec<Request>>>,
    pub(crate) bytes: usize,
}

#[async_trait]
impl Provider for LargeToolCallProvider {
    fn metadata(&self) -> Metadata {
        Metadata {
            provider: Arc::from("large-tool-provider"),
            model: "large-tool-model".to_string(),
            budget: Some(Cap {
                bytes: self.bytes,
                source: "test".to_string(),
            }),
        }
    }

    async fn stream(&self, request: Request) -> Result<Streaming, String> {
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
            let raw = arguments.to_string();
            return Ok(Box::pin(stream::iter(vec![
                Ok(Event::Text("assistant before tool".to_string())),
                Ok(Event::Called(Call {
                    response: "resp_large_tool".to_string(),
                    mark: Some("item_large_tool".to_string()),
                    item: json!({
                        "type": "function_call",
                        "id": "item_large_tool",
                        "call_id": "call_large_tool",
                        "name": "shell",
                        "arguments": raw,
                    }),
                    call: "call_large_tool".to_string(),
                    name: "shell".to_string(),
                    raw,
                    arguments,
                })),
                Ok(Event::Completed {
                    response: Some("resp_large_tool".to_string()),
                }),
            ])));
        }
        Ok(Box::pin(stream::iter(vec![Ok(Event::Completed {
            response: Some("resp_after_large_tool".to_string()),
        })])))
    }
}

#[derive(Clone)]
pub(crate) struct GatedFirstProvider {
    pub(crate) requests: Arc<Mutex<Vec<Request>>>,
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
impl Provider for GatedFirstProvider {
    fn metadata(&self) -> Metadata {
        Metadata {
            provider: Arc::from("gated-provider"),
            model: "gated-model".to_string(),
            budget: None,
        }
    }

    async fn stream(&self, request: Request) -> Result<Streaming, String> {
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
            Ok(Event::Text(format!("provider response {index}"))),
            Ok(Event::Completed {
                response: Some(format!("gated-response-{index}")),
            }),
        ])))
    }
}

fn probe_command() -> &'static str {
    if cfg!(windows) {
        "[Console]::Out.WriteLine((Get-Location).Path); [Console]::Out.WriteLine($env:SANTI_STRAND_MEMORY_DIR); [Console]::Out.WriteLine($env:SANTI_SOUL_ID); [Console]::Out.WriteLine($env:SANTI_STRAND_ID); [Console]::Out.WriteLine($env:SANTI_TURN_ID)"
    } else {
        "pwd && printf \"\\n$SANTI_STRAND_MEMORY_DIR\\n$SANTI_SOUL_ID\\n$SANTI_STRAND_ID\\n$SANTI_TURN_ID\""
    }
}
