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
    CompactExecRequest, CompactExecResponse, CompactQueryResponse, CreateSoulRequest,
    CreateStrandResponse, CreateWebhookRequest, IngestOutcome, MaterialKind, MessageContent,
    MessageKind, SantiStore, SantiStreamEvent, SantiStreamPayload, SendStrandAcceptedResponse,
    SendStrandRequest, Soul, Strand, StrandDetail, StrandMaterial, StrandMessage,
    StrandRuntimeSnapshot, StrandSelector, ThinkingCompletionReason, ThinkingSpan, Turn,
    TurnActivityState, WebhookSubscription, prefixed_id, timestamp_now,
};
use failure::ProviderTurnFailure;
use text_delta::TextDeltaUpdate;
use timing::{ProviderTurnTiming, provider_event_name};

#[derive(Clone)]
pub struct SantiService {
    pub(crate) store: SantiStore,
    provider: Arc<dyn ProviderClient>,
    pub(crate) config: SantiServiceConfig,
    material_cache: Arc<Mutex<HashMap<MaterialCacheKey, StrandMaterial>>>,
    stream_events: broadcast::Sender<SantiStreamEvent>,
}

type MaterialCacheKey = (String, MaterialKind);
/// A turn `poke`/`ingest_into` actually just drove, with what it drained into
/// the timeline to reach it — `None` when nothing was pending, or the drive
/// coalesced into an already-running turn instead of starting a fresh one.
type DrivenTurn = Option<(Turn, Vec<StrandMessage>)>;

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
        // sees the truth and its strand is idle again. Re-driving stranded
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

    /// Re-drive strands left "behind" by a crash (their inbox durably holds
    /// content nobody ever drained). Liveness only — no retry of
    /// attempted/failed turns. Call once at server startup (inside the tokio
    /// runtime).
    pub fn resume_pending(&self) {
        match self.store.strands_with_pending_requests() {
            Ok(pending) => {
                for strand_id in pending {
                    self.poke(&strand_id, "strand_send");
                }
            }
            Err(error) => eprintln!("santi: resume_pending scan failed: {error}"),
        }
    }

    pub fn subscribe_stream(&self) -> broadcast::Receiver<SantiStreamEvent> {
        self.stream_events.subscribe()
    }

    pub fn create_strand(&self) -> Result<CreateStrandResponse, String> {
        Ok(CreateStrandResponse {
            strand: self.store.create_strand()?,
        })
    }

    /// Create a soul and seed its initial `[santi-soul]` memory. A soul is
    /// id-only; its identity IS its memory, so creation optionally carries the
    /// starting memory to write into the soul's memory file (absent → a blank
    /// soul that will author its own).
    pub fn create_soul(&self, request: CreateSoulRequest) -> Result<Soul, String> {
        let soul = self.store.create_soul()?;
        if let Some(memory) = request
            .memory
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty())
        {
            let path = self.soul_memory_file(&soul.id);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            std::fs::write(&path, memory).map_err(|error| error.to_string())?;
        }
        Ok(soul)
    }

    pub fn list_souls(&self) -> Result<Vec<Soul>, String> {
        self.store.list_souls()
    }

    pub fn soul(&self, soul_id: &str) -> Result<Option<Soul>, String> {
        self.store.soul(soul_id)
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
        if self.store.soul(soul_id)?.is_none() {
            return Err("soul not found".to_string());
        }
        let strand_strategy = request
            .strand_strategy
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("per_thread");
        if !matches!(strand_strategy, "per_thread" | "single") {
            return Err("strand_strategy must be 'per_thread' or 'single'".to_string());
        }
        self.store
            .create_webhook(name, adaptor, soul_id, strand_strategy, secret_env)
    }

    pub fn list_webhooks(&self) -> Result<Vec<WebhookSubscription>, String> {
        self.store.list_webhooks()
    }

    pub fn webhook(&self, name: &str) -> Result<Option<WebhookSubscription>, String> {
        self.store.webhook(name)
    }

    /// The one inbound path (see PHASE-06/STEP4): resolve `selector` to a
    /// strand, enqueue `content` into its durable inbox, try to drive a turn.
    /// `Accepted` only confirms durable enqueue, not that a message/turn now
    /// exists yet — the driver may still be draining a running turn's inbox
    /// later (see `ingest_into`). `Rejected` (the inbox gate — a scale safety
    /// valve, not an error) is a normal outcome; handling it is the adaptor's
    /// own policy (surface it, or silently drop + log).
    pub fn ingest(
        &self,
        selector: StrandSelector,
        content: MessageContent,
        kind: MessageKind,
        trigger_type: &str,
    ) -> Result<IngestOutcome, String> {
        let strand = self.store.resolve_strand_selector(&selector)?;
        let (outcome, _driven) = self.ingest_into(&strand, content, kind, trigger_type)?;
        Ok(outcome)
    }

    /// Shared ingest core (enqueue + drive) for both the generic `ingest` and
    /// `send_strand` (which additionally wants the turn/message it may have
    /// just driven, to shape its richer response). Returns `driven = Some` only
    /// when THIS call's poke actually drained the inbox (a fresh turn started,
    /// possibly covering other adaptors' concurrently-enqueued entries too) —
    /// `None` when it coalesced into an already-running turn, whose own
    /// completion re-check will drain this content later.
    fn ingest_into(
        &self,
        strand: &Strand,
        content: MessageContent,
        kind: MessageKind,
        trigger_type: &str,
    ) -> Result<(IngestOutcome, DrivenTurn), String> {
        let outcome = self.store.enqueue_inbox(&strand.id, kind, content)?;
        let driven = match outcome {
            IngestOutcome::Accepted { .. } => self.poke(&strand.id, trigger_type),
            IngestOutcome::Rejected { .. } => None,
        };
        Ok((outcome, driven))
    }

    /// Ingest an external event already normalized by an adaptor: a `santi-system`
    /// message addressed to `soul_id`, anchored to the strand bound to `label`.
    /// This is the webhook twin of `send_strand` — same `ingest_into` core, so
    /// the same drive/coalesce/gate semantics. Core stays generic: the label and
    /// the message text are opaque (the adaptor owns their meaning).
    pub fn ingest_external_event(
        &self,
        soul_id: &str,
        label: &str,
        system_text: String,
    ) -> Result<IngestOutcome, String> {
        let strand = self
            .store
            .resolve_strand_selector(&StrandSelector::ByLabel {
                soul_id: soul_id.to_string(),
                label: label.to_string(),
            })?;
        let (outcome, _driven) = self.ingest_into(
            &strand,
            MessageContent::text(system_text),
            MessageKind::SantiSystem,
            "system",
        )?;
        Ok(outcome)
    }

    /// Compact a range of a strand's own timeline (self-involved: the soul
    /// runs this on itself). Creates the projection overlay directly over the
    /// addressed strand. The soul authors `summary`; the system only checks scale.
    pub fn compact_exec(
        &self,
        strand_id: &str,
        request: CompactExecRequest,
    ) -> Result<CompactExecResponse, String> {
        let from = request.from_message_id.trim();
        let to = request.to_message_id.trim();
        let summary = request.summary.trim();
        if from.is_empty() || to.is_empty() {
            return Err("compact requires from_message_id and to_message_id".to_string());
        }
        if summary.is_empty() {
            return Err("compact summary must not be empty".to_string());
        }
        let strand = self
            .store
            .strand(strand_id)?
            .ok_or_else(|| "strand not found".to_string())?;
        self.store.create_compact(&strand.id, from, to, summary)
    }

    pub fn compact_query(
        &self,
        compact_id: &str,
        keyword: Option<&str>,
        page_index: i64,
        page_size: i64,
    ) -> Result<Option<CompactQueryResponse>, String> {
        self.store
            .compact_query(compact_id, keyword, page_index, page_size)
    }

    pub fn list_strands(&self) -> Result<Vec<Strand>, String> {
        self.store.list_strands()
    }

    pub fn strand(&self, strand_id: &str) -> Result<Option<StrandDetail>, String> {
        let Some(strand) = self.store.strand(strand_id)? else {
            return Ok(None);
        };
        Ok(Some(StrandDetail {
            messages: self.store.strand_messages(strand_id)?,
            strand,
        }))
    }

    pub fn runtime_snapshot(
        &self,
        strand_id: &str,
    ) -> Result<Option<StrandRuntimeSnapshot>, String> {
        self.store.runtime_snapshot(strand_id)
    }

    pub async fn send_strand(
        &self,
        strand_id: &str,
        request: SendStrandRequest,
    ) -> Result<SendStrandAcceptedResponse, String> {
        let text = request.text();
        if text.trim().is_empty() {
            return Err("send content must contain text".to_string());
        }
        let strand = self
            .store
            .strand(strand_id)?
            .ok_or_else(|| "strand not found".to_string())?;

        // ingest_into is decoupled from drive: enqueue into the inbox, then try
        // to drive a turn. If one is already running for this strand, this send
        // simply joins the thread (coalesced) and the running turn (or its
        // completion re-check) will drain it later — no second concurrent turn.
        let (outcome, driven) = self.ingest_into(
            &strand,
            MessageContent {
                parts: request.content,
            },
            MessageKind::Text,
            "strand_send",
        )?;
        if let IngestOutcome::Rejected { reason } = outcome {
            return Err(reason);
        }
        let (turn, user_message) = match driven {
            Some((turn, mut drained)) => (turn, drained.pop()),
            None => (
                self.store
                    .latest_turn(&strand.id)?
                    .ok_or_else(|| "no active turn after send".to_string())?,
                None,
            ),
        };

        Ok(SendStrandAcceptedResponse {
            strand,
            turn,
            user_message,
        })
    }

    /// Drive a turn if the strand is behind (its inbox is non-empty) and idle,
    /// spawning the runner. Returns the started turn plus what it drained into
    /// the timeline, or None when a turn is already running (this request
    /// coalesces) or there is nothing pending. The atomic guard in
    /// `try_start_turn` keeps "one present per thread of experience".
    fn poke(&self, strand_id: &str, trigger_type: &str) -> DrivenTurn {
        match self.store.try_start_turn(strand_id, trigger_type, None) {
            Ok(Some(started)) => {
                for message in started.drained_messages.iter().cloned() {
                    self.publish_stream(strand_id, SantiStreamPayload::MessageCreated { message });
                }
                self.publish_stream(
                    strand_id,
                    SantiStreamPayload::TurnStarted {
                        turn: started.turn.clone(),
                    },
                );
                let background = self.clone();
                let background_strand_id = strand_id.to_string();
                let background_turn_id = started.turn.id.clone();
                tokio::spawn(async move {
                    background
                        .complete_provider_turn(background_strand_id, background_turn_id)
                        .await;
                });
                Some((started.turn, started.drained_messages))
            }
            Ok(None) => None,
            Err(error) => {
                eprintln!("santi: try_start_turn failed for {strand_id}: {error}");
                None
            }
        }
    }

    async fn complete_provider_turn(&self, strand_id: String, turn_id: String) {
        match self.run_provider_turn(&strand_id, &turn_id).await {
            Err(failure) => {
                self.fail_background_turn(
                    &strand_id,
                    &turn_id,
                    failure.error,
                    failure.partial_assistant_text,
                );
            }
            Ok((last_soul_message, provider_response_id)) => {
                self.finalize_turn(
                    &strand_id,
                    &turn_id,
                    last_soul_message,
                    provider_response_id,
                );
            }
        }
        // Re-check: a turn is one thread "catching up"; requests that arrived
        // during it (seq past this turn's start) make the strand behind
        // again → drive the next turn now.
        self.poke(&strand_id, "strand_send");
    }

    /// Finalize a completed provider turn. Speech is optional (N6): an empty
    /// turn (no per-round text ever appended) is a valid silent completion, not
    /// a failure. `last_soul_message` is the final per-round entry `run_provider_turn`
    /// appended (if any) — already the operator-visible truth, so completion just
    /// marks the turn done, it does not write anything new.
    fn finalize_turn(
        &self,
        strand_id: &str,
        turn_id: &str,
        last_soul_message: Option<StrandMessage>,
        provider_response_id: Option<String>,
    ) {
        let assistant_seq = last_soul_message.map(|message| {
            let seq = message.relation.strand_seq;
            self.publish_stream(
                strand_id,
                SantiStreamPayload::MessageCompleted {
                    turn_id: turn_id.to_string(),
                    message,
                },
            );
            seq
        });
        match self.store.complete_turn(
            turn_id,
            assistant_seq,
            &self.provider.metadata().provider,
            provider_response_id,
        ) {
            Ok(_) => self.publish_stream(
                strand_id,
                SantiStreamPayload::TurnCompleted {
                    turn_id: turn_id.to_string(),
                },
            ),
            Err(error) => self.fail_background_turn(strand_id, turn_id, error, String::new()),
        }
    }

    async fn run_provider_turn(
        &self,
        strand_id: &str,
        turn_id: &str,
    ) -> Result<(Option<StrandMessage>, Option<String>), ProviderTurnFailure> {
        let mut assistant_text = String::new();
        let mut last_soul_message: Option<StrandMessage> = None;
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
            let input = provider_try!(provider_input(&self.store, strand_id));
            let metadata = self.provider.metadata();
            let request = ProviderRequest {
                model: metadata.model,
                instructions: Some(provider_try!(self.system_prompt_text(strand_id))),
                input,
                tools: Some(provider_tools()),
                previous_response_id: None,
            };
            timing.request_built(
                round,
                request.input.len(),
                request.instructions.as_ref().map_or(0, |text| text.len()),
            );
            self.publish_turn_activity(strand_id, turn_id, TurnActivityState::Requesting, None);
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
                            strand_id,
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
                            strand_id,
                            turn_id,
                            &mut current_thinking_span,
                            &mut summary_thinking_span,
                            provider_response_id.clone(),
                        ));
                        self.publish_turn_activity(
                            strand_id,
                            turn_id,
                            TurnActivityState::Thinking,
                            provider_response_id,
                        );
                    }
                    ProviderEvent::ReasoningSummaryDelta(delta) => {
                        reasoning_summary.push_str(&delta);
                        provider_try!(self.update_thinking_span_summary(
                            strand_id,
                            &mut summary_thinking_span,
                            reasoning_summary.clone(),
                        ));
                    }
                    ProviderEvent::ReasoningSummaryDone(summary) => {
                        reasoning_summary = summary;
                        provider_try!(self.update_thinking_span_summary(
                            strand_id,
                            &mut summary_thinking_span,
                            reasoning_summary.clone(),
                        ));
                    }
                    ProviderEvent::TextDelta(delta) => {
                        let update = TextDeltaUpdate {
                            strand_id,
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
                            strand_id,
                            &mut current_thinking_span,
                            ThinkingCompletionReason::ToolCallRequested,
                        ));
                        self.publish_turn_activity(
                            strand_id,
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
                            strand_id,
                            &mut current_thinking_span,
                            ThinkingCompletionReason::ProviderCompleted,
                        ));
                        completed_response_id = provider_response_id;
                        break;
                    }
                    ProviderEvent::Failed(error) => {
                        provider_try!(self.fail_current_thinking_span(
                            strand_id,
                            &mut current_thinking_span,
                            error.clone(),
                        ));
                        return Err(ProviderTurnFailure::new(error, &assistant_text));
                    }
                }
            }

            // Persist this round's assistant text as a timeline item before its
            // tool calls (or as the final item), so the replay timeline stays a
            // faithful interleaved log (DC4b). The lumped strand-visible reply is
            // stored once at turn end.
            if !round_assistant_text.is_empty() {
                last_soul_message = Some(provider_try!(
                    self.store
                        .append_soul_assistant_text(strand_id, &round_assistant_text)
                ));
            }

            if calls.is_empty() {
                break completed_response_id;
            }

            timing.tool_outputs_started(round, calls.len());
            let call_count = calls.len();
            for call in calls {
                self.publish_turn_activity(
                    strand_id,
                    turn_id,
                    TurnActivityState::RunningTool,
                    active_provider_response_id.clone(),
                );
                provider_try!(self.handle_tool_call(strand_id, turn_id, call));
            }
            timing.tool_outputs_completed(round, call_count);
        };

        Ok((last_soul_message, final_response_id))
    }

    pub(crate) fn publish_stream(&self, strand_id: &str, payload: SantiStreamPayload) {
        let _ = self.stream_events.send(SantiStreamEvent {
            event_id: prefixed_id("stream"),
            strand_id: strand_id.to_string(),
            created_at: timestamp_now(),
            payload,
        });
    }
}
