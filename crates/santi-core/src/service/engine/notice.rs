use std::{
    collections::{HashSet, VecDeque},
    sync::{Arc, Mutex},
};

use santi_provider::Item;

use super::{Service, address::Address};
use crate::{message, stream};

pub(in crate::service) struct Observation<'a> {
    pub(in crate::service) address: Address<&'a str>,
    pub(in crate::service) round: usize,
    pub(in crate::service) provider: &'a str,
    pub(in crate::service) model: &'a str,
    pub(in crate::service) input: &'a [Item],
    pub(in crate::service) instructions: Option<&'a str>,
}

const NOTICES: usize = 128;
pub(in crate::service) const REFERENCE: usize = 96 * 1024;

#[derive(Debug, Clone)]
pub(in crate::service) enum Event {
    Observed(Observed),
}

impl Event {
    fn turn(&self) -> &str {
        match self {
            Self::Observed(event) => &event.address.turn,
        }
    }

    fn dedupe(&self) -> Option<String> {
        match self {
            Self::Observed(event) => event.dedupe(),
        }
    }
}

#[derive(Debug, Clone)]
pub(in crate::service) struct Observed {
    pub(in crate::service) address: Address<String>,
    pub(in crate::service) round: usize,
    pub(in crate::service) provider: String,
    pub(in crate::service) model: String,
    pub(in crate::service) items: usize,
    pub(in crate::service) input: usize,
    pub(in crate::service) instructions: usize,
    pub(in crate::service) threshold: usize,
    pub(in crate::service) band: String,
}

impl Observed {
    fn total(&self) -> usize {
        self.input.saturating_add(self.instructions)
    }

    fn remindable(&self) -> bool {
        self.total() >= self.threshold
    }

    fn dedupe(&self) -> Option<String> {
        self.remindable().then(|| {
            format!(
                "compact_reminder:{}:{}:{}",
                self.address.strand, self.threshold, self.band
            )
        })
    }
}

#[derive(Clone, Debug)]
pub(in crate::service) struct Bus {
    inner: Arc<Mutex<State>>,
}

#[derive(Debug)]
struct State {
    queue: VecDeque<Event>,
    held: HashSet<String>,
    capacity: usize,
}

impl Bus {
    pub(in crate::service) fn new() -> Self {
        Self::sized(NOTICES)
    }

    fn sized(capacity: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(State {
                queue: VecDeque::new(),
                held: HashSet::new(),
                capacity,
            })),
        }
    }

    pub(in crate::service) fn publish(&self, event: Event) -> bool {
        let dedupe = event.dedupe();
        let mut state = self.inner.lock().unwrap();
        if let Some(key) = dedupe.as_ref()
            && state.held.contains(key)
        {
            return false;
        }
        if state.queue.len() >= state.capacity {
            return false;
        }
        if let Some(key) = dedupe {
            state.held.insert(key);
        }
        state.queue.push_back(event);
        true
    }

    pub(in crate::service) fn drained(&self, turn: &str) -> Vec<Event> {
        let mut state = self.inner.lock().unwrap();
        let mut drained = Vec::new();
        let mut kept = VecDeque::with_capacity(state.queue.len());
        while let Some(event) = state.queue.pop_front() {
            if event.turn() == turn {
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
    pub(in crate::service) fn observed(&self, observation: Observation<'_>) {
        let input = heft(observation.input);
        let instructions = observation.instructions.map_or(0, str::len);
        let event = Observed {
            address: observation.address.owned(),
            round: observation.round,
            provider: observation.provider.to_string(),
            model: observation.model.to_string(),
            items: observation.input.len(),
            input,
            instructions,
            threshold: REFERENCE,
            band: "soft".to_string(),
        };
        let _ = self.notices.publish(Event::Observed(event));
    }

    pub(in crate::service) fn noticed(&self, turn: &str) {
        for event in self.notices.drained(turn) {
            if let Err(error) = self.absorbed(event) {
                eprintln!("santi: internal runtime notice failed: {error}");
            }
        }
    }

    fn absorbed(&self, event: Event) -> Result<(), String> {
        match event {
            Event::Observed(event) => self.remind(event),
        }
    }

    fn remind(&self, event: Observed) -> Result<(), String> {
        if !event.remindable() {
            return Ok(());
        }
        let content = reminded(&event);
        let message = self.store.append_santi_system_message(
            &event.address.strand,
            content,
            message::Intake::Record,
        )?;
        self.publish(
            &event.address.strand,
            stream::Payload::MessageCreated {
                message: message.strand_message,
            },
        );
        Ok(())
    }
}

fn reminded(event: &Observed) -> message::Content {
    message::Content::text(
        [
            "<system_message>".to_string(),
            "kind: compact_reminder".to_string(),
            "scope: strand_local".to_string(),
            "wake: false".to_string(),
            "obligation: false".to_string(),
            format!("trigger_turn_id: {}", event.address.turn),
            format!("round: {}", event.round),
            format!("provider: {}", event.provider),
            format!("model: {}", event.model),
            format!("items: {}", event.items),
            format!("input: {}", event.input),
            format!("instructions: {}", event.instructions),
            format!("total_input_bytes: {}", event.total()),
            format!(
                "reference_threshold_bytes: {}",
                event.threshold
            ),
            "summary: This strand is getting large. If useful, you may compact settled context; runtime did not compact or alter provider input.".to_string(),
            "</system_message>".to_string(),
        ]
        .join("\n"),
    )
}

fn heft(input: &[Item]) -> usize {
    input.iter().map(sized).sum()
}

fn sized(item: &Item) -> usize {
    match item {
        Item::Message { role, content } => role.len().saturating_add(content.len()),
        Item::Reasoning { id, content } => id
            .as_ref()
            .map_or(0, String::len)
            .saturating_add(content.len()),
        Item::Call {
            call,
            name,
            raw,
            item,
            mark,
        } => call
            .len()
            .saturating_add(name.len())
            .saturating_add(raw.len())
            .saturating_add(mark.as_ref().map_or(0, String::len))
            .saturating_add(item.as_ref().map_or(0, |value| value.to_string().len())),
        Item::Output { call, output } => call.len().saturating_add(output.len()),
    }
}
