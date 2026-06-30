mod failure;
mod materials;
mod text_delta;
mod thinking;
mod timing;
mod tools;

use futures_util::StreamExt;
use santi_provider::{ProviderClient, ProviderEvent, ProviderRequest};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::sync::broadcast;

use crate::assembly::input::provider_messages;
use crate::service_prompt::provider_tools;
use crate::{
    ActorType, CreateSessionResponse, MaterialKind, MessageContent, MessageState, SantiStore,
    SantiStreamEvent, SantiStreamPayload, SendSessionAcceptedResponse, SendSessionRequest,
    SessionDetail, SessionMaterial, SessionRuntimeSnapshot, SessionSummary,
    ThinkingCompletionReason, ThinkingSpan, TurnActivityState, UpdateSessionRequest, prefixed_id,
    timestamp_now,
};
use failure::ProviderTurnFailure;
use text_delta::TextDeltaUpdate;
use timing::{ProviderTurnTiming, provider_event_name};

#[derive(Clone)]
pub struct SantiService {
    pub(crate) store: SantiStore,
    provider: Arc<dyn ProviderClient>,
    pub(crate) config: SantiServiceConfig,
    material_cache: Arc<Mutex<HashMap<MaterialCacheKey, SessionMaterial>>>,
    stream_events: broadcast::Sender<SantiStreamEvent>,
}

type MaterialCacheKey = (String, MaterialKind);

#[derive(Debug, Clone)]
pub struct SantiServiceConfig {
    pub database_path: String,
    pub runtime_root: String,
    pub execution_root: String,
    pub bind_addr: Option<String>,
}

impl SantiService {
    pub fn open(
        config: SantiServiceConfig,
        provider: Arc<dyn ProviderClient>,
    ) -> Result<Self, String> {
        let store = SantiStore::open(&config.database_path)?;
        Ok(Self {
            store,
            provider,
            config,
            material_cache: Arc::new(Mutex::new(HashMap::new())),
            stream_events: broadcast::channel(1024).0,
        })
    }

    pub fn subscribe_stream(&self) -> broadcast::Receiver<SantiStreamEvent> {
        self.stream_events.subscribe()
    }

    pub fn create_session(&self) -> Result<CreateSessionResponse, String> {
        Ok(CreateSessionResponse {
            session: self.store.create_session()?,
        })
    }

    pub fn list_sessions(&self) -> Result<Vec<SessionSummary>, String> {
        self.store.list_sessions()
    }

    pub fn session(&self, session_id: &str) -> Result<Option<SessionDetail>, String> {
        let Some(session) = self.store.session(session_id)? else {
            return Ok(None);
        };
        Ok(Some(SessionDetail {
            profile: self
                .store
                .runtime_snapshot(session_id)?
                .map(|snapshot| snapshot.profile)
                .ok_or_else(|| "session disappeared".to_string())?,
            session,
            messages: self.store.session_messages(session_id)?,
        }))
    }

    pub fn update_session(
        &self,
        session_id: &str,
        request: UpdateSessionRequest,
    ) -> Result<Option<SessionSummary>, String> {
        self.store.update_session_title(session_id, request.title)
    }

    pub fn runtime_snapshot(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionRuntimeSnapshot>, String> {
        self.store.runtime_snapshot(session_id)
    }

    pub async fn send_session(
        &self,
        session_id: &str,
        request: SendSessionRequest,
    ) -> Result<SendSessionAcceptedResponse, String> {
        let text = request.text();
        if text.trim().is_empty() {
            return Err("send content must contain text".to_string());
        }

        let user_message = self
            .store
            .append_message(
                session_id,
                ActorType::Account,
                self.store.default_account_id(),
                MessageContent {
                    parts: request.content,
                },
                MessageState::Fixed,
            )?
            .session_message;
        let soul_session = self.store.acquire_soul_session(session_id)?.soul_session;
        self.store
            .append_message_ref(&soul_session.id, &user_message.message.id)?;
        self.publish_stream(
            session_id,
            SantiStreamPayload::MessageCreated {
                message: user_message.clone(),
            },
        );
        let turn = self
            .store
            .start_turn(
                &soul_session.id,
                &user_message.message.id,
                user_message.relation.session_seq,
            )?
            .turn;
        self.publish_stream(
            session_id,
            SantiStreamPayload::TurnStarted { turn: turn.clone() },
        );

        let snapshot = self
            .store
            .runtime_snapshot(session_id)?
            .ok_or_else(|| "soul_session disappeared".to_string())?;
        let accepted_soul_session = snapshot
            .soul_session
            .ok_or_else(|| "soul_session disappeared".to_string())?;
        let soul_profile = snapshot
            .soul_profile
            .ok_or_else(|| "soul_profile disappeared".to_string())?;
        let background = self.clone();
        let background_session_id = session_id.to_string();
        let background_soul_session_id = soul_session.id.clone();
        let background_turn_id = turn.id.clone();
        tokio::spawn(async move {
            background
                .complete_provider_turn(
                    background_session_id,
                    background_soul_session_id,
                    background_turn_id,
                )
                .await;
        });

        Ok(SendSessionAcceptedResponse {
            session: SessionSummary {
                session: snapshot.session,
                profile: snapshot.profile,
            },
            soul_session: accepted_soul_session,
            soul_profile,
            turn,
            user_message,
        })
    }

    async fn complete_provider_turn(
        &self,
        session_id: String,
        soul_session_id: String,
        turn_id: String,
    ) {
        let send_result = self
            .run_provider_turn(&session_id, &soul_session_id, &turn_id)
            .await;

        let (assistant_text, provider_response_id) = match send_result {
            Ok(value) => value,
            Err(failure) => {
                self.fail_background_turn(
                    &session_id,
                    &turn_id,
                    failure.error,
                    failure.partial_assistant_text,
                );
                return;
            }
        };

        if assistant_text.trim().is_empty() {
            let error = "provider completed without assistant output".to_string();
            self.fail_background_turn(&session_id, &turn_id, error, String::new());
            return;
        }

        let assistant_message = match self.store.append_message(
            &session_id,
            ActorType::Soul,
            self.store.default_soul_id(),
            MessageContent::text(assistant_text.clone()),
            MessageState::Fixed,
        ) {
            Ok(message) => message.session_message,
            Err(error) => {
                self.fail_background_turn(&session_id, &turn_id, error, assistant_text);
                return;
            }
        };
        if let Err(error) = self
            .store
            .append_message_ref(&soul_session_id, &assistant_message.message.id)
        {
            self.fail_background_turn(&session_id, &turn_id, error, String::new());
            return;
        }
        if let Err(error) = self.store.complete_turn(
            &turn_id,
            assistant_message.relation.session_seq,
            &self.provider.metadata().provider,
            provider_response_id,
        ) {
            self.fail_background_turn(&session_id, &turn_id, error, String::new());
            return;
        }
        self.publish_stream(
            &session_id,
            SantiStreamPayload::MessageCompleted {
                turn_id,
                message: assistant_message,
            },
        );
    }

    async fn run_provider_turn(
        &self,
        session_id: &str,
        soul_session_id: &str,
        turn_id: &str,
    ) -> Result<(String, Option<String>), ProviderTurnFailure> {
        let mut assistant_text = String::new();
        let mut function_call_outputs = Vec::new();
        let mut timing = ProviderTurnTiming::new(turn_id);
        let mut round = 0;
        macro_rules! provider_try {
            ($expr:expr) => {
                match $expr {
                    Ok(value) => value,
                    Err(error) => return Err(ProviderTurnFailure::new(error, &assistant_text)),
                }
            };
        }

        let tools_through_seq = provider_try!(self.store.turn_base_soul_session_seq(turn_id));
        let final_response_id = loop {
            round += 1;
            let input = provider_try!(provider_messages(
                &self.store,
                soul_session_id,
                tools_through_seq
            ));
            let metadata = self.provider.metadata();
            let request = ProviderRequest {
                model: metadata.model,
                instructions: Some(provider_try!(
                    self.system_prompt_text(session_id, soul_session_id)
                )),
                input,
                tools: Some(provider_tools()),
                previous_response_id: None,
                function_call_outputs: if function_call_outputs.is_empty() {
                    None
                } else {
                    Some(function_call_outputs.clone())
                },
            };
            timing.request_built(
                round,
                request.input.len(),
                request.instructions.as_ref().map_or(0, |text| text.len()),
                request
                    .function_call_outputs
                    .as_ref()
                    .map_or(0, |outputs| outputs.len()),
            );
            self.publish_turn_activity(session_id, turn_id, TurnActivityState::Requesting, None);
            let mut stream = match self.provider.stream_response(request).await {
                Ok(stream) => {
                    timing.http_response_started(round);
                    stream
                }
                Err(error) => {
                    timing.failed(round, "http_response", &error);
                    return Err(ProviderTurnFailure::new(error, &assistant_text));
                }
            };
            let mut calls = Vec::new();
            let mut completed_response_id = None;
            let mut active_provider_response_id = None;
            let mut current_thinking_span: Option<ThinkingSpan> = None;
            let mut summary_thinking_span: Option<ThinkingSpan> = None;
            let mut reasoning_summary = String::new();
            let mut round_assistant_text = String::new();
            let mut saw_sse_event = false;

            while let Some(event) = stream.next().await {
                let event = match event {
                    Ok(event) => event,
                    Err(error) => {
                        timing.failed(round, "sse_event", &error);
                        provider_try!(self.fail_current_thinking_span(
                            session_id,
                            &mut current_thinking_span,
                            error.clone(),
                        ));
                        return Err(ProviderTurnFailure::new(error, &assistant_text));
                    }
                };
                if let ProviderEvent::StreamTrace(trace) = event {
                    timing.provider_trace(round, trace);
                    continue;
                }
                if !saw_sse_event {
                    saw_sse_event = true;
                    timing.first_sse_event(round, provider_event_name(&event));
                }
                match event {
                    ProviderEvent::StreamTrace(_) => {}
                    ProviderEvent::ResponseStarted {
                        provider_response_id,
                    }
                    | ProviderEvent::ResponseInProgress {
                        provider_response_id,
                    } => {
                        active_provider_response_id = provider_response_id.clone();
                        provider_try!(self.ensure_thinking_span(
                            session_id,
                            turn_id,
                            &mut current_thinking_span,
                            &mut summary_thinking_span,
                            provider_response_id.clone(),
                        ));
                        self.publish_turn_activity(
                            session_id,
                            turn_id,
                            TurnActivityState::Thinking,
                            provider_response_id,
                        );
                    }
                    ProviderEvent::ReasoningSummaryDelta(delta) => {
                        reasoning_summary.push_str(&delta);
                        provider_try!(self.update_thinking_span_summary(
                            session_id,
                            &mut summary_thinking_span,
                            reasoning_summary.clone(),
                        ));
                    }
                    ProviderEvent::ReasoningSummaryDone(summary) => {
                        reasoning_summary = summary;
                        provider_try!(self.update_thinking_span_summary(
                            session_id,
                            &mut summary_thinking_span,
                            reasoning_summary.clone(),
                        ));
                    }
                    ProviderEvent::TextDelta(delta) => {
                        let update = TextDeltaUpdate {
                            session_id,
                            turn_id,
                            assistant_text: &mut assistant_text,
                            round_assistant_text: &mut round_assistant_text,
                            timing: &timing,
                            round,
                            current_thinking_span: &mut current_thinking_span,
                            active_provider_response_id: &active_provider_response_id,
                        };
                        provider_try!(self.handle_text_delta(delta, update));
                    }
                    ProviderEvent::FunctionCallRequested(call) => {
                        timing.function_call_requested(round, &call.name);
                        provider_try!(self.complete_current_thinking_span(
                            session_id,
                            &mut current_thinking_span,
                            ThinkingCompletionReason::ToolCallRequested,
                        ));
                        self.publish_turn_activity(
                            session_id,
                            turn_id,
                            TurnActivityState::CallingTool,
                            active_provider_response_id.clone(),
                        );
                        calls.push(call);
                    }
                    ProviderEvent::Completed {
                        provider_response_id,
                    } => {
                        timing.completed(round);
                        active_provider_response_id = provider_response_id.clone();
                        provider_try!(self.complete_current_thinking_span(
                            session_id,
                            &mut current_thinking_span,
                            ThinkingCompletionReason::ProviderCompleted,
                        ));
                        completed_response_id = provider_response_id;
                        break;
                    }
                    ProviderEvent::Failed(error) => {
                        provider_try!(self.fail_current_thinking_span(
                            session_id,
                            &mut current_thinking_span,
                            error.clone(),
                        ));
                        return Err(ProviderTurnFailure::new(error, &assistant_text));
                    }
                }
            }

            if calls.is_empty() {
                break completed_response_id;
            }

            let mut outputs = Vec::new();
            timing.tool_outputs_started(round, calls.len());
            for call in calls {
                self.publish_turn_activity(
                    session_id,
                    turn_id,
                    TurnActivityState::RunningTool,
                    active_provider_response_id.clone(),
                );
                let mut output = provider_try!(self.handle_tool_call(
                    session_id,
                    soul_session_id,
                    turn_id,
                    call
                ));
                if !round_assistant_text.is_empty() {
                    output.assistant_content = Some(round_assistant_text.clone());
                }
                if !reasoning_summary.is_empty() {
                    output.reasoning_content = Some(reasoning_summary.clone());
                }
                outputs.push(output);
            }
            timing.tool_outputs_completed(round, outputs.len());
            function_call_outputs.extend(outputs);
        };

        Ok((assistant_text, final_response_id))
    }

    pub(crate) fn publish_stream(&self, session_id: &str, payload: SantiStreamPayload) {
        let _ = self.stream_events.send(SantiStreamEvent {
            event_id: prefixed_id("stream"),
            session_id: session_id.to_string(),
            created_at: timestamp_now(),
            payload,
        });
    }
}
