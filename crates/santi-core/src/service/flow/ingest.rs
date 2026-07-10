use crate::{
    InboxSource, IngestOutcome, MessageContent, MessageKind, SantiStreamPayload,
    SendStrandAcceptedResponse, SendStrandRequest, Strand, StrandSelector,
};

use super::super::{DrivenTurn, SantiService};

impl SantiService {
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
        self.ingest_external_source(soul_id, label, system_text, None)
    }

    pub fn ingest_external_source(
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
    pub(in crate::service) fn poke(&self, strand_id: &str, trigger_type: &str) -> DrivenTurn {
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
}
