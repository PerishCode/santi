use std::{
    collections::{HashSet, VecDeque},
    sync::{Arc, Mutex},
};

use santi_provider::ProviderItem;

use crate::{MessageContent, MessageIntake, SantiStreamPayload};

use super::{Service, address::Address};

pub(super) struct Observation<'a> {
    pub(super) address: Address<&'a str>,
    pub(super) round: usize,
    pub(super) provider: &'a str,
    pub(super) model: &'a str,
    pub(super) input: &'a [ProviderItem],
    pub(super) instructions: Option<&'a str>,
}

const RUNTIME_NOTICE_QUEUE_CAPACITY: usize = 128;
pub(super) const COMPACT_REMINDER_REFERENCE_BYTES: usize = 96 * 1024;

#[derive(Debug, Clone)]
pub(super) enum Event {
    Observed(Observed),
}

impl Event {
    fn turn_id(&self) -> &str {
        match self {
            Self::Observed(event) => &event.address.turn_id,
        }
    }

    fn dedupe_key(&self) -> Option<String> {
        match self {
            Self::Observed(event) => event.dedupe_key(),
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct Observed {
    pub(super) address: Address<String>,
    pub(super) round: usize,
    pub(super) provider: String,
    pub(super) model: String,
    pub(super) input_items: usize,
    pub(super) input_bytes: usize,
    pub(super) instructions_bytes: usize,
    pub(super) reference_threshold_bytes: usize,
    pub(super) band: String,
}

impl Observed {
    fn total_input_bytes(&self) -> usize {
        self.input_bytes.saturating_add(self.instructions_bytes)
    }

    fn should_remind(&self) -> bool {
        self.total_input_bytes() >= self.reference_threshold_bytes
    }

    fn dedupe_key(&self) -> Option<String> {
        self.should_remind().then(|| {
            format!(
                "compact_reminder:{}:{}:{}",
                self.address.strand_id, self.reference_threshold_bytes, self.band
            )
        })
    }
}

#[derive(Clone, Debug)]
pub(super) struct Bus {
    inner: Arc<Mutex<State>>,
}

#[derive(Debug)]
struct State {
    queue: VecDeque<Event>,
    queued_or_delivered_keys: HashSet<String>,
    capacity: usize,
}

impl Bus {
    pub(super) fn new() -> Self {
        Self::with_capacity(RUNTIME_NOTICE_QUEUE_CAPACITY)
    }

    fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(State {
                queue: VecDeque::new(),
                queued_or_delivered_keys: HashSet::new(),
                capacity,
            })),
        }
    }

    pub(super) fn publish(&self, event: Event) -> bool {
        let dedupe_key = event.dedupe_key();
        let mut state = self.inner.lock().unwrap();
        if let Some(key) = dedupe_key.as_ref()
            && state.queued_or_delivered_keys.contains(key)
        {
            return false;
        }
        if state.queue.len() >= state.capacity {
            return false;
        }
        if let Some(key) = dedupe_key {
            state.queued_or_delivered_keys.insert(key);
        }
        state.queue.push_back(event);
        true
    }

    pub(super) fn drain_for_turn(&self, turn_id: &str) -> Vec<Event> {
        let mut state = self.inner.lock().unwrap();
        let mut drained = Vec::new();
        let mut kept = VecDeque::with_capacity(state.queue.len());
        while let Some(event) = state.queue.pop_front() {
            if event.turn_id() == turn_id {
                drained.push(event);
            } else {
                kept.push_back(event);
            }
        }
        state.queue = kept;
        drained
    }
}

impl Default for Bus {
    fn default() -> Self {
        Self::new()
    }
}

impl Service {
    pub(super) fn observe_provider_input(&self, observation: Observation<'_>) {
        let input_bytes = provider_input_bytes(observation.input);
        let instructions_bytes = observation.instructions.map_or(0, str::len);
        let event = Observed {
            address: observation.address.owned(),
            round: observation.round,
            provider: observation.provider.to_string(),
            model: observation.model.to_string(),
            input_items: observation.input.len(),
            input_bytes,
            instructions_bytes,
            reference_threshold_bytes: COMPACT_REMINDER_REFERENCE_BYTES,
            band: "soft".to_string(),
        };
        let _ = self.runtime_notices.publish(Event::Observed(event));
    }

    pub(super) fn drain_runtime_notices(&self, turn_id: &str) {
        for event in self.runtime_notices.drain_for_turn(turn_id) {
            if let Err(error) = self.handle_internal_runtime_event(event) {
                eprintln!("santi: internal runtime notice failed: {error}");
            }
        }
    }

    fn handle_internal_runtime_event(&self, event: Event) -> Result<(), String> {
        match event {
            Event::Observed(event) => self.maybe_materialize_compact_reminder(event),
        }
    }

    fn maybe_materialize_compact_reminder(&self, event: Observed) -> Result<(), String> {
        if !event.should_remind() {
            return Ok(());
        }
        let content = compact_reminder_message(&event);
        let message = self.store.append_santi_system_message(
            &event.address.strand_id,
            content,
            MessageIntake::Record,
        )?;
        self.publish_stream(
            &event.address.strand_id,
            SantiStreamPayload::MessageCreated {
                message: message.strand_message,
            },
        );
        Ok(())
    }
}

fn compact_reminder_message(event: &Observed) -> MessageContent {
    MessageContent::text(
        [
            "<system_message>".to_string(),
            "kind: compact_reminder".to_string(),
            "scope: strand_local".to_string(),
            "wake: false".to_string(),
            "obligation: false".to_string(),
            format!("trigger_turn_id: {}", event.address.turn_id),
            format!("round: {}", event.round),
            format!("provider: {}", event.provider),
            format!("model: {}", event.model),
            format!("input_items: {}", event.input_items),
            format!("input_bytes: {}", event.input_bytes),
            format!("instructions_bytes: {}", event.instructions_bytes),
            format!("total_input_bytes: {}", event.total_input_bytes()),
            format!(
                "reference_threshold_bytes: {}",
                event.reference_threshold_bytes
            ),
            "summary: This strand is getting large. If useful, you may compact settled context; runtime did not compact or alter provider input.".to_string(),
            "</system_message>".to_string(),
        ]
        .join("\n"),
    )
}

fn provider_input_bytes(input: &[ProviderItem]) -> usize {
    input.iter().map(provider_item_bytes).sum()
}

fn provider_item_bytes(item: &ProviderItem) -> usize {
    match item {
        ProviderItem::Message { role, content } => role.len().saturating_add(content.len()),
        ProviderItem::Reasoning { id, content } => id
            .as_ref()
            .map_or(0, String::len)
            .saturating_add(content.len()),
        ProviderItem::FunctionCall {
            call_id,
            name,
            arguments_raw,
            item,
            item_id,
        } => call_id
            .len()
            .saturating_add(name.len())
            .saturating_add(arguments_raw.len())
            .saturating_add(item_id.as_ref().map_or(0, String::len))
            .saturating_add(item.as_ref().map_or(0, |value| value.to_string().len())),
        ProviderItem::FunctionCallOutput { call_id, output } => {
            call_id.len().saturating_add(output.len())
        }
    }
}
