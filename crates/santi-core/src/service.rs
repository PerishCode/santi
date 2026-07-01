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

use crate::assembly::input::provider_input;
use crate::service_prompt::provider_tools;
use crate::{
    ActorType, CreateSessionResponse, CreateSoulRequest, CreateWebhookRequest, MaterialKind,
    MessageContent, MessageIntake, MessageState, SantiStore, SantiStreamEvent, SantiStreamPayload,
    SendSessionAcceptedResponse, SendSessionRequest, SessionDetail, SessionMaterial,
    SessionRuntimeSnapshot, SessionSummary, SoulProfile, ThinkingCompletionReason, ThinkingSpan,
    Turn, TurnActivityState, UpdateSessionRequest, WebhookSubscription, prefixed_id, timestamp_now,
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
        // Boot recovery (honest occurrence): any turn still `running` is orphaned
        // by the restart — reconcile it to an interrupted terminal so the soul
        // sees the truth and its soul_session is idle again. Re-driving stranded
        // requests is liveness; call `resume_pending` once inside the runtime.
        store.reconcile_orphaned_turns()?;
        Ok(Self {
            store,
            provider,
            config,
            material_cache: Arc::new(Mutex::new(HashMap::new())),
            stream_events: broadcast::channel(1024).0,
        })
    }

    /// Re-drive soul_sessions left "behind" by a crash (durable requests never
    /// covered by a turn). Liveness only — no retry of attempted/failed turns.
    /// Call once at server startup (inside the tokio runtime).
    pub fn resume_pending(&self) {
        match self.store.soul_sessions_with_pending_requests() {
            Ok(pending) => {
                for (session_id, soul_session_id) in pending {
                    self.poke(&session_id, &soul_session_id, "session_send");
                }
            }
            Err(error) => eprintln!("santi: resume_pending scan failed: {error}"),
        }
    }

    pub fn subscribe_stream(&self) -> broadcast::Receiver<SantiStreamEvent> {
        self.stream_events.subscribe()
    }

    pub fn create_session(&self) -> Result<CreateSessionResponse, String> {
        Ok(CreateSessionResponse {
            session: self.store.create_session()?,
        })
    }

    pub fn create_soul(&self, request: CreateSoulRequest) -> Result<SoulProfile, String> {
        if request.soul_name.trim().is_empty() {
            return Err("soul_name must not be empty".to_string());
        }
        self.store.create_soul(
            request.soul_name.trim(),
            request.nickname.trim(),
            request
                .desc
                .as_deref()
                .map(str::trim)
                .filter(|d| !d.is_empty()),
        )
    }

    pub fn list_souls(&self) -> Result<Vec<SoulProfile>, String> {
        self.store.list_souls()
    }

    pub fn soul(&self, soul_id: &str) -> Result<Option<SoulProfile>, String> {
        self.store.soul_profile(soul_id)
    }

    pub fn create_webhook(
        &self,
        request: CreateWebhookRequest,
    ) -> Result<WebhookSubscription, String> {
        let name = request.name.trim();
        let adaptor = request.adaptor.trim();
        let soul_id = request.soul_id.trim();
        let secret_env = request.secret_env.trim();
        if name.is_empty() {
            return Err("webhook name must not be empty".to_string());
        }
        if adaptor.is_empty() {
            return Err("webhook adaptor must not be empty".to_string());
        }
        if secret_env.is_empty() {
            return Err("webhook secret_env must not be empty".to_string());
        }
        if self.store.soul_profile(soul_id)?.is_none() {
            return Err("soul not found".to_string());
        }
        let session_strategy = request
            .session_strategy
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("per_thread");
        if !matches!(session_strategy, "per_thread" | "single") {
            return Err("session_strategy must be 'per_thread' or 'single'".to_string());
        }
        self.store
            .create_webhook(name, adaptor, soul_id, session_strategy, secret_env)
    }

    pub fn list_webhooks(&self) -> Result<Vec<WebhookSubscription>, String> {
        self.store.list_webhooks()
    }

    pub fn webhook(&self, name: &str) -> Result<Option<WebhookSubscription>, String> {
        self.store.webhook(name)
    }

    /// Ingest an external event already normalized by an adaptor: a `santi-system`
    /// message addressed to `soul_id`, anchored to the session bound to `label`.
    /// This is the webhook twin of `send_session` — append a REQUEST + poke the
    /// driver — so it shares the same drive/coalesce semantics. Core stays generic:
    /// the label and the message text are opaque (the adaptor owns their meaning).
    /// Returns the anchored session id.
    pub fn ingest_external_event(
        &self,
        soul_id: &str,
        label: &str,
        system_text: String,
    ) -> Result<String, String> {
        if self.store.soul_profile(soul_id)?.is_none() {
            return Err("soul not found".to_string());
        }
        let session = self.store.find_or_create_session_by_label(label)?;
        let session_id = session.session.id;
        let system_message = self
            .store
            .append_santi_system_message(
                &session_id,
                MessageContent::text(system_text),
                MessageIntake::Request,
            )?
            .session_message;
        let soul_session = self
            .store
            .acquire_soul_session(soul_id, &session_id)?
            .soul_session;
        self.store
            .append_message_ref(&soul_session.id, &system_message.message.id)?;
        self.publish_stream(
            &session_id,
            SantiStreamPayload::MessageCreated {
                message: system_message,
            },
        );
        self.poke(&session_id, &soul_session.id, "system");
        Ok(session_id)
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
                .runtime_snapshot(self.store.default_soul_id(), session_id)?
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
        // Session-level view from the default soul's vantage (the pre-multi-soul
        // shortcut the GET /runtime endpoint still uses).
        self.store
            .runtime_snapshot(self.store.default_soul_id(), session_id)
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

        // Ingest is decoupled from drive: append the user message as a REQUEST,
        // then poke the driver. If a turn is already running for this soul_session,
        // this send simply joins the thread (coalesced) and the running turn (or
        // its completion re-check) will see it — no second concurrent turn.
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
                MessageIntake::Request,
            )?
            .session_message;
        let soul_id = request
            .soul_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .unwrap_or_else(|| self.store.default_soul_id());
        let soul_session = self
            .store
            .acquire_soul_session(soul_id, session_id)?
            .soul_session;
        self.store
            .append_message_ref(&soul_session.id, &user_message.message.id)?;
        self.publish_stream(
            session_id,
            SantiStreamPayload::MessageCreated {
                message: user_message.clone(),
            },
        );
        let turn = match self.poke(session_id, &soul_session.id, "session_send") {
            Some(turn) => turn,
            None => self
                .store
                .latest_turn(&soul_session.id)?
                .ok_or_else(|| "no active turn after send".to_string())?,
        };

        let snapshot = self
            .store
            .runtime_snapshot(soul_id, session_id)?
            .ok_or_else(|| "soul_session disappeared".to_string())?;
        let accepted_soul_session = snapshot
            .soul_session
            .ok_or_else(|| "soul_session disappeared".to_string())?;
        let soul_profile = snapshot
            .soul_profile
            .ok_or_else(|| "soul_profile disappeared".to_string())?;

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

    /// Drive a turn if the soul_session is behind and idle, spawning the runner.
    /// Returns the started turn, or None when a turn is already running (this
    /// request coalesces) or there is nothing pending. The atomic guard in
    /// `try_start_turn` keeps "one present per thread of experience".
    fn poke(&self, session_id: &str, soul_session_id: &str, trigger_type: &str) -> Option<Turn> {
        match self
            .store
            .try_start_turn(soul_session_id, trigger_type, None)
        {
            Ok(Some(turn)) => {
                self.publish_stream(
                    session_id,
                    SantiStreamPayload::TurnStarted { turn: turn.clone() },
                );
                let background = self.clone();
                let background_session_id = session_id.to_string();
                let background_soul_session_id = soul_session_id.to_string();
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
                Some(turn)
            }
            Ok(None) => None,
            Err(error) => {
                eprintln!("santi: try_start_turn failed for {soul_session_id}: {error}");
                None
            }
        }
    }

    async fn complete_provider_turn(
        &self,
        session_id: String,
        soul_session_id: String,
        turn_id: String,
    ) {
        match self
            .run_provider_turn(&session_id, &soul_session_id, &turn_id)
            .await
        {
            Err(failure) => {
                self.fail_background_turn(
                    &session_id,
                    &turn_id,
                    failure.error,
                    failure.partial_assistant_text,
                );
            }
            Ok((assistant_text, provider_response_id)) => {
                let soul_id = self
                    .store
                    .soul_id_for_soul_session(&soul_session_id)
                    .unwrap_or_else(|_| self.store.default_soul_id().to_string());
                self.finalize_turn(
                    &session_id,
                    &turn_id,
                    &soul_id,
                    assistant_text,
                    provider_response_id,
                );
            }
        }
        // Re-check: a turn is one thread "catching up"; requests that arrived
        // during it (seq past this turn's start) make the soul_session behind
        // again → drive the next turn now.
        self.poke(&session_id, &soul_session_id, "session_send");
    }

    /// Finalize a completed provider turn. Speech is optional (N6): an empty
    /// assistant_text is a valid silent completion, not a failure. The lumped,
    /// user-visible reply is a RECORD (it does not wake the soul); the replay
    /// timeline already holds this turn's per-round output.
    fn finalize_turn(
        &self,
        session_id: &str,
        turn_id: &str,
        soul_id: &str,
        assistant_text: String,
        provider_response_id: Option<String>,
    ) {
        let assistant_seq = if assistant_text.trim().is_empty() {
            None
        } else {
            match self.store.append_message(
                session_id,
                ActorType::Soul,
                soul_id,
                MessageContent::text(assistant_text.clone()),
                MessageState::Fixed,
                MessageIntake::Record,
            ) {
                Ok(message) => {
                    let assistant_message = message.session_message;
                    let seq = assistant_message.relation.session_seq;
                    self.publish_stream(
                        session_id,
                        SantiStreamPayload::MessageCompleted {
                            turn_id: turn_id.to_string(),
                            message: assistant_message,
                        },
                    );
                    Some(seq)
                }
                Err(error) => {
                    self.fail_background_turn(session_id, turn_id, error, assistant_text);
                    return;
                }
            }
        };
        match self.store.complete_turn(
            turn_id,
            assistant_seq,
            &self.provider.metadata().provider,
            provider_response_id,
        ) {
            Ok(_) => self.publish_stream(
                session_id,
                SantiStreamPayload::TurnCompleted {
                    turn_id: turn_id.to_string(),
                },
            ),
            Err(error) => self.fail_background_turn(session_id, turn_id, error, String::new()),
        }
    }

    async fn run_provider_turn(
        &self,
        session_id: &str,
        soul_session_id: &str,
        turn_id: &str,
    ) -> Result<(String, Option<String>), ProviderTurnFailure> {
        let mut assistant_text = String::new();
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

        let final_response_id = loop {
            round += 1;
            // The timeline is the single source of truth: each round re-derives
            // input from it, including any tool calls/results just persisted by
            // the previous round (no function_call_outputs side-channel).
            let input = provider_try!(provider_input(&self.store, soul_session_id));
            let metadata = self.provider.metadata();
            let request = ProviderRequest {
                model: metadata.model,
                instructions: Some(provider_try!(
                    self.system_prompt_text(session_id, soul_session_id)
                )),
                input,
                tools: Some(provider_tools()),
                previous_response_id: None,
            };
            timing.request_built(
                round,
                request.input.len(),
                request.instructions.as_ref().map_or(0, |text| text.len()),
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

            // Persist this round's assistant text as a timeline item before its
            // tool calls (or as the final item), so the replay timeline stays a
            // faithful interleaved log (DC4b). The lumped session-visible reply is
            // stored once at turn end.
            if !round_assistant_text.is_empty() {
                provider_try!(
                    self.store
                        .append_soul_assistant_text(soul_session_id, &round_assistant_text)
                );
            }

            if calls.is_empty() {
                break completed_response_id;
            }

            timing.tool_outputs_started(round, calls.len());
            let call_count = calls.len();
            for call in calls {
                self.publish_turn_activity(
                    session_id,
                    turn_id,
                    TurnActivityState::RunningTool,
                    active_provider_response_id.clone(),
                );
                provider_try!(self.handle_tool_call(session_id, soul_session_id, turn_id, call));
            }
            timing.tool_outputs_completed(round, call_count);
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
