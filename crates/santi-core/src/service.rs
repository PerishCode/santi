mod budget;
mod failure;
mod fork;
mod im;
mod materials;
mod runtime_notice;
mod text_delta;
mod thinking;
mod timing;
mod tools;

use futures_util::StreamExt;
use santi_provider::{ProviderClient, ProviderEvent, ProviderRequest};
use serde_json::json;
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::broadcast;

use crate::assembly::input::provider_input;
use crate::context_budget::{estimate_provider_parts, estimate_provider_request};
use crate::service_prompt::provider_tools;
use crate::{
    CompactCapsuleOptions, CompactExecRequest, CompactExecResponse, CompactQueryResponse,
    ContextBudget, ContextEstimate, CreateSoulRequest, CreateStrandResponse, CreateWebhookRequest,
    InboxSource, IngestOutcome, MaterialKind, MessageContent, MessageKind, RejectedDelivery,
    SantiStore, SantiStreamEvent, SantiStreamPayload, SendStrandAcceptedResponse,
    SendStrandRequest, Soul, Strand, StrandBudgetSnapshot, StrandDetail, StrandMaterial,
    StrandMessage, StrandRuntimeSnapshot, StrandSelector, ThinkingCompletionReason, ThinkingSpan,
    Turn, TurnActivityState, WebhookSubscription, prefixed_id, timestamp_now,
};
use failure::ProviderTurnFailure;
use runtime_notice::{ProviderInputObservation, RuntimeNoticeBus};
use text_delta::TextDeltaUpdate;
use timing::{ProviderTurnTiming, provider_event_name};

#[derive(Clone)]
pub struct SantiService {
    pub(crate) store: SantiStore,
    provider: Arc<dyn ProviderClient>,
    pub(crate) config: SantiServiceConfig,
    material_cache: Arc<Mutex<HashMap<MaterialCacheKey, StrandMaterial>>>,
    stream_events: broadcast::Sender<SantiStreamEvent>,
    runtime_notices: RuntimeNoticeBus,
    /// Graceful-shutdown latch (PHASE-07): once set, `poke` refuses to START new
    /// turns, so inbox CONSUMPTION pauses while ingest keeps durably enqueuing
    /// (the inbox is an MQ — we stop consuming, never producing). The in-flight
    /// turn is left to finish; `drain_running_turns` waits it out.
    shutting_down: Arc<AtomicBool>,
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
            runtime_notices: RuntimeNoticeBus::new(),
            shutting_down: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Begin a graceful shutdown: stop consuming the inbox (no new turns start).
    /// Idempotent. Ingest still durably enqueues; the in-flight turn finishes.
    pub fn begin_shutdown(&self) {
        self.shutting_down.store(true, Ordering::SeqCst);
    }

    pub fn is_shutting_down(&self) -> bool {
        self.shutting_down.load(Ordering::SeqCst)
    }

    /// Wait until no turn is `running` (the in-flight turn finished), or until
    /// `cap` elapses. Called after the HTTP server has stopped accepting, so no
    /// new turns can appear once shutdown has begun. On cap-timeout it returns
    /// anyway: the still-running turn will be reconciled to `interrupted` on the
    /// next boot (honest occurrence), and the external upgrade flow's own bound
    /// (SIGKILL) is the hard stop.
    pub async fn drain_running_turns(&self, cap: Duration) {
        let start = Instant::now();
        loop {
            match self.store.running_turn_count() {
                Ok(0) => return,
                Ok(remaining) => {
                    if start.elapsed() >= cap {
                        eprintln!(
                            "santi: shutdown drain cap reached with {remaining} turn(s) still running; leaving them to boot-recovery"
                        );
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
                Err(error) => {
                    eprintln!("santi: shutdown drain scan failed: {error}");
                    return;
                }
            }
        }
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
        self.ingest_with_source(selector, content, kind, trigger_type, None)
    }

    pub fn ingest_with_source(
        &self,
        selector: StrandSelector,
        content: MessageContent,
        kind: MessageKind,
        trigger_type: &str,
        source: Option<InboxSource>,
    ) -> Result<IngestOutcome, String> {
        let strand = self.store.resolve_strand_selector(&selector)?;
        let (outcome, _driven) = self.ingest_into(&strand, content, kind, trigger_type, source)?;
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
        source: Option<InboxSource>,
    ) -> Result<(IngestOutcome, DrivenTurn), String> {
        let admission = self.context_admission(&strand.id)?;
        let outcome = self.store.enqueue_inbox_with_context(
            &strand.id,
            kind,
            content,
            source,
            admission.as_ref(),
        )?;
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
        self.ingest_external_event_with_source(soul_id, label, system_text, None)
    }

    pub fn ingest_external_event_with_source(
        &self,
        soul_id: &str,
        label: &str,
        system_text: String,
        source: Option<InboxSource>,
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
            source,
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
        let summary = request.summary.trim();
        if summary.is_empty() {
            return Err("compact summary must not be empty".to_string());
        }
        let strand = self
            .store
            .strand(strand_id)?
            .ok_or_else(|| "strand not found".to_string())?;
        let (from, to) = self.resolve_compact_boundaries(&strand.id, &request)?;
        let pre_estimate = self.current_context_estimate(&strand.id)?;
        if request.dry_run {
            let mut response = self.store.preview_compact(&strand.id, &from, &to)?;
            response.pre_estimate = Some(pre_estimate);
            if let Some(capsule) = request.capsule.as_ref() {
                let metadata = compact_capsule_metadata(CompactCapsuleMetadataInput {
                    compact_id: Some(&response.compact_id),
                    capsule,
                    response: Some(&response),
                    pre_estimate: response.pre_estimate.as_ref(),
                    post_estimate: None,
                    budget: self.context_budget().as_ref(),
                    compression_ratio: None,
                });
                let post_estimate =
                    self.estimate_preview_compact(&strand.id, &response, summary, metadata)?;
                let compression_ratio = compact_compression_ratio(
                    response.pre_estimate.as_ref().unwrap(),
                    &post_estimate,
                );
                let metadata = compact_capsule_metadata(CompactCapsuleMetadataInput {
                    compact_id: Some(&response.compact_id),
                    capsule,
                    response: Some(&response),
                    pre_estimate: response.pre_estimate.as_ref(),
                    post_estimate: Some(&post_estimate),
                    budget: self.context_budget().as_ref(),
                    compression_ratio,
                });
                let post_estimate =
                    self.estimate_preview_compact(&strand.id, &response, summary, metadata)?;
                response.compression_ratio = compact_compression_ratio(
                    response.pre_estimate.as_ref().unwrap(),
                    &post_estimate,
                );
                response.post_estimate = Some(post_estimate);
            }
            return Ok(response);
        }

        let initial_metadata = request.capsule.as_ref().map(|capsule| {
            compact_capsule_metadata(CompactCapsuleMetadataInput {
                compact_id: None,
                capsule,
                response: None,
                pre_estimate: Some(&pre_estimate),
                post_estimate: None,
                budget: self.context_budget().as_ref(),
                compression_ratio: None,
            })
        });
        let mut response = self.store.create_compact_with_metadata(
            &strand.id,
            &from,
            &to,
            summary,
            initial_metadata,
        )?;
        let mut post_estimate = self.current_context_estimate(&strand.id)?;
        let mut compression_ratio = compact_compression_ratio(&pre_estimate, &post_estimate);
        if let Some(capsule) = request.capsule.as_ref() {
            let metadata = compact_capsule_metadata(CompactCapsuleMetadataInput {
                compact_id: Some(&response.compact_id),
                capsule,
                response: Some(&response),
                pre_estimate: Some(&pre_estimate),
                post_estimate: Some(&post_estimate),
                budget: self.context_budget().as_ref(),
                compression_ratio,
            });
            self.store
                .update_compact_metadata(&response.compact_id, metadata)?;
            post_estimate = self.current_context_estimate(&strand.id)?;
            compression_ratio = compact_compression_ratio(&pre_estimate, &post_estimate);
            let metadata = compact_capsule_metadata(CompactCapsuleMetadataInput {
                compact_id: Some(&response.compact_id),
                capsule,
                response: Some(&response),
                pre_estimate: Some(&pre_estimate),
                post_estimate: Some(&post_estimate),
                budget: self.context_budget().as_ref(),
                compression_ratio,
            });
            self.store
                .update_compact_metadata(&response.compact_id, metadata)?;
        }
        response.active_block_cleared = self.clear_context_block(&strand.id, "compact_exec")?;
        response.pre_estimate = Some(pre_estimate);
        response.post_estimate = Some(post_estimate);
        response.compression_ratio = compression_ratio;
        Ok(response)
    }

    fn resolve_compact_boundaries(
        &self,
        strand_id: &str,
        request: &CompactExecRequest,
    ) -> Result<(String, String), String> {
        let from_id = request
            .from_message_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let to_id = request
            .to_message_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        match (from_id, to_id, request.from_seq, request.to_seq) {
            (Some(from), Some(to), None, None) => Ok((from.to_string(), to.to_string())),
            (None, None, Some(from_seq), Some(to_seq)) => {
                let from = self
                    .store
                    .message_id_at_seq(strand_id, from_seq)?
                    .ok_or_else(|| format!("compact from_seq {from_seq} is not a message"))?;
                let to = self
                    .store
                    .message_id_at_seq(strand_id, to_seq)?
                    .ok_or_else(|| format!("compact to_seq {to_seq} is not a message"))?;
                Ok((from, to))
            }
            _ => Err(
                "compact requires either from_message_id/to_message_id or from_seq/to_seq"
                    .to_string(),
            ),
        }
    }

    fn estimate_preview_compact(
        &self,
        strand_id: &str,
        response: &CompactExecResponse,
        summary: &str,
        metadata: serde_json::Value,
    ) -> Result<ContextEstimate, String> {
        let input = self
            .store
            .assembly_input_preview(strand_id, response, summary, metadata)?;
        let instructions = self.system_prompt_text(strand_id)?;
        let tools = provider_tools();
        Ok(estimate_provider_parts(
            &input,
            Some(&instructions),
            Some(&tools),
        ))
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

    pub fn strand_budget(&self, strand_id: &str) -> Result<Option<StrandBudgetSnapshot>, String> {
        let Some(strand) = self.store.strand(strand_id)? else {
            return Ok(None);
        };
        Ok(Some(StrandBudgetSnapshot {
            strand_id: strand.id.clone(),
            estimate: self.current_context_estimate(&strand.id)?,
            budget: self.context_budget(),
            active_block: self.store.active_context_block(&strand.id)?,
        }))
    }

    pub fn strand_rejections(
        &self,
        strand_id: &str,
        limit: i64,
    ) -> Result<Option<Vec<RejectedDelivery>>, String> {
        let Some(strand) = self.store.strand(strand_id)? else {
            return Ok(None);
        };
        self.store.rejected_deliveries(&strand.id, limit).map(Some)
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
            Some(InboxSource::new("strand_send").with_ref(strand.id.clone())),
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
        // Graceful shutdown: pause CONSUMPTION. The content stays durably in the
        // inbox (ingest already enqueued it) and boot recovery drains it on the
        // next start. This also stops the completion re-poke from spawning a
        // follow-on turn, so an in-flight turn can finish and the strand quiesce.
        if self.is_shutting_down() {
            return None;
        }
        let admission = match self.context_admission(strand_id) {
            Ok(admission) => admission,
            Err(error) => {
                eprintln!("santi: pending context-budget gate failed for {strand_id}: {error}");
                return None;
            }
        };
        match self
            .store
            .start_turn_with_budget(strand_id, trigger_type, None, admission.as_ref())
        {
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
                    failure.record_failure_message,
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
        self.drain_internal_runtime_notices_for_turn(&turn_id);
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
            Err(error) => self.fail_background_turn(strand_id, turn_id, error, String::new(), true),
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
            let provider_family = metadata.provider.to_string();
            let request = ProviderRequest {
                model: metadata.model,
                instructions: Some(provider_try!(self.system_prompt_text(strand_id))),
                input,
                tools: Some(provider_tools()),
                previous_response_id: None,
            };
            let estimate = estimate_provider_request(&request);
            if let Some(reason) = provider_try!(
                self.block_over_budget_request(strand_id, turn_id, &request, &estimate)
            ) {
                timing.failed(round, "context_budget", &reason);
                return Err(ProviderTurnFailure::context_budget(reason));
            }
            timing.request_built(
                round,
                request.input.len(),
                request.instructions.as_ref().map_or(0, |text| text.len()),
            );
            self.observe_provider_input_for_notices(ProviderInputObservation {
                strand_id,
                turn_id,
                round,
                provider: &provider_family,
                model: &request.model,
                input: &request.input,
                instructions: request.instructions.as_deref(),
            });
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

struct CompactCapsuleMetadataInput<'a> {
    compact_id: Option<&'a str>,
    capsule: &'a CompactCapsuleOptions,
    response: Option<&'a CompactExecResponse>,
    pre_estimate: Option<&'a ContextEstimate>,
    post_estimate: Option<&'a ContextEstimate>,
    budget: Option<&'a ContextBudget>,
    compression_ratio: Option<f64>,
}

const CAPSULE_SOURCE_BYTES: usize = 128;
const CAPSULE_REASON_BYTES: usize = 512;
const CAPSULE_RISK_BYTES: usize = 1024;
const CAPSULE_QUERYABILITY_BYTES: usize = 512;

fn compact_capsule_metadata(input: CompactCapsuleMetadataInput<'_>) -> serde_json::Value {
    let originals_query = input
        .compact_id
        .map(|id| format!("santi compact query --compact-id {id}"));
    let range = input.response.map(|response| {
        json!({
            "start_seq": response.start_seq,
            "end_seq": response.end_seq,
            "start_message_id": response.start_message_id,
            "end_message_id": response.end_message_id,
            "collapsed_count": response.collapsed_count,
            "absorbed": response.absorbed,
        })
    });
    json!({
        "schema": "santi.compact_capsule.v1",
        "operation": "manual_capsule",
        "compact_id": input.compact_id,
        "declared_source": cap_capsule_field(&input.capsule.source, CAPSULE_SOURCE_BYTES),
        "source_trust": "caller_declared",
        "reason": cap_capsule_field(&input.capsule.reason, CAPSULE_REASON_BYTES),
        "risk": cap_capsule_field(&input.capsule.risk, CAPSULE_RISK_BYTES),
        "queryability": input.capsule.queryability.as_ref().map(|value| {
            cap_capsule_field(value, CAPSULE_QUERYABILITY_BYTES)
        }),
        "originals_query": originals_query,
        "range": range,
        "pre_estimate": input.pre_estimate,
        "post_estimate": input.post_estimate,
        "budget": input.budget,
        "compression_ratio": input.compression_ratio,
    })
}

fn compact_compression_ratio(
    pre_estimate: &ContextEstimate,
    post_estimate: &ContextEstimate,
) -> Option<f64> {
    if pre_estimate.total_bytes <= 0 {
        return None;
    }
    Some(post_estimate.total_bytes as f64 / pre_estimate.total_bytes as f64)
}

fn cap_capsule_field(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let suffix = " [truncated]";
    let suffix_bytes = suffix.len();
    let mut end = max_bytes.saturating_sub(suffix_bytes).min(value.len());
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{}", &value[..end], suffix)
}
