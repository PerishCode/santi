use std::{
    collections::{HashSet, VecDeque},
    sync::{Arc, Mutex},
};

use santi_provider::ProviderItem;

use crate::{MessageContent, MessageIntake, SantiStreamPayload};

use super::SantiService;

pub(super) struct ProviderInputObservation<'a> {
    pub(super) strand_id: &'a str,
    pub(super) turn_id: &'a str,
    pub(super) round: usize,
    pub(super) provider: &'a str,
    pub(super) model: &'a str,
    pub(super) input: &'a [ProviderItem],
    pub(super) instructions: Option<&'a str>,
}

const RUNTIME_NOTICE_QUEUE_CAPACITY: usize = 128;
pub(super) const COMPACT_REMINDER_REFERENCE_BYTES: usize = 96 * 1024;

#[derive(Debug, Clone)]
pub(super) enum RuntimeEvent {
    ProviderInputObserved(ProviderInputObserved),
}

impl RuntimeEvent {
    fn turn_id(&self) -> &str {
        match self {
            Self::ProviderInputObserved(event) => &event.turn_id,
        }
    }

    fn dedupe_key(&self) -> Option<String> {
        match self {
            Self::ProviderInputObserved(event) => event.dedupe_key(),
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct ProviderInputObserved {
    pub(super) strand_id: String,
    pub(super) turn_id: String,
    pub(super) round: usize,
    pub(super) provider: String,
    pub(super) model: String,
    pub(super) input_items: usize,
    pub(super) input_bytes: usize,
    pub(super) instructions_bytes: usize,
    pub(super) reference_threshold_bytes: usize,
    pub(super) band: String,
}

impl ProviderInputObserved {
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
                self.strand_id, self.reference_threshold_bytes, self.band
            )
        })
    }
}

#[derive(Clone, Debug)]
pub(super) struct RuntimeNoticeBus {
    inner: Arc<Mutex<RuntimeNoticeState>>,
}

#[derive(Debug)]
struct RuntimeNoticeState {
    queue: VecDeque<RuntimeEvent>,
    queued_or_delivered_keys: HashSet<String>,
    capacity: usize,
}

impl RuntimeNoticeBus {
    pub(super) fn new() -> Self {
        Self::with_capacity(RUNTIME_NOTICE_QUEUE_CAPACITY)
    }

    fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(RuntimeNoticeState {
                queue: VecDeque::new(),
                queued_or_delivered_keys: HashSet::new(),
                capacity,
            })),
        }
    }

    pub(super) fn publish(&self, event: RuntimeEvent) -> bool {
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

    pub(super) fn drain_for_turn(&self, turn_id: &str) -> Vec<RuntimeEvent> {
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

impl Default for RuntimeNoticeBus {
    fn default() -> Self {
        Self::new()
    }
}

impl SantiService {
    pub(super) fn observe_provider_input_for_notices(
        &self,
        observation: ProviderInputObservation<'_>,
    ) {
        let input_bytes = provider_input_bytes(observation.input);
        let instructions_bytes = observation.instructions.map_or(0, str::len);
        let event = ProviderInputObserved {
            strand_id: observation.strand_id.to_string(),
            turn_id: observation.turn_id.to_string(),
            round: observation.round,
            provider: observation.provider.to_string(),
            model: observation.model.to_string(),
            input_items: observation.input.len(),
            input_bytes,
            instructions_bytes,
            reference_threshold_bytes: COMPACT_REMINDER_REFERENCE_BYTES,
            band: "soft".to_string(),
        };
        // Ordinary compact reminders are hygiene notices. Losing or coalescing
        // this in-memory event must not affect the provider call.
        let _ = self
            .runtime_notices
            .publish(RuntimeEvent::ProviderInputObserved(event));
    }

    pub(super) fn drain_internal_runtime_notices_for_turn(&self, turn_id: &str) {
        for event in self.runtime_notices.drain_for_turn(turn_id) {
            if let Err(error) = self.handle_internal_runtime_event(event) {
                eprintln!("santi: internal runtime notice failed: {error}");
            }
        }
    }

    fn handle_internal_runtime_event(&self, event: RuntimeEvent) -> Result<(), String> {
        match event {
            RuntimeEvent::ProviderInputObserved(event) => {
                self.maybe_materialize_compact_reminder(event)
            }
        }
    }

    fn maybe_materialize_compact_reminder(
        &self,
        event: ProviderInputObserved,
    ) -> Result<(), String> {
        if !event.should_remind() {
            return Ok(());
        }
        let content = compact_reminder_message(&event);
        let message = self.store.append_santi_system_message(
            &event.strand_id,
            content,
            MessageIntake::Record,
        )?;
        self.publish_stream(
            &event.strand_id,
            SantiStreamPayload::MessageCreated {
                message: message.strand_message,
            },
        );
        Ok(())
    }
}

fn compact_reminder_message(event: &ProviderInputObserved) -> MessageContent {
    MessageContent::text(
        [
            "<system_message>".to_string(),
            "kind: compact_reminder".to_string(),
            "scope: strand_local".to_string(),
            "wake: false".to_string(),
            "obligation: false".to_string(),
            format!("trigger_turn_id: {}", event.turn_id),
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use santi_provider::{
        ProviderClient, ProviderItem, ProviderMetadata, ProviderRequest, ProviderStream,
    };

    use super::*;
    use crate::{MessageKind, SantiServiceConfig};

    struct UnusedProvider;

    #[async_trait]
    impl ProviderClient for UnusedProvider {
        fn metadata(&self) -> ProviderMetadata {
            ProviderMetadata {
                provider: Arc::from("test"),
                model: "test-model".to_string(),
            }
        }

        async fn stream_response(
            &self,
            _request: ProviderRequest,
        ) -> Result<ProviderStream, String> {
            Err("unused provider".to_string())
        }
    }

    fn service() -> (SantiService, tempfile::TempDir) {
        let temp = tempfile::tempdir().expect("temp dir");
        let service = SantiService::open(
            SantiServiceConfig {
                database_path: temp.path().join("db").to_string_lossy().to_string(),
                runtime_root: temp.path().join("runtime").to_string_lossy().to_string(),
                execution_root: temp.path().join("exec").to_string_lossy().to_string(),
                bind_addr: None,
            },
            Arc::new(UnusedProvider),
        )
        .expect("open service");
        (service, temp)
    }

    fn observed_event(strand_id: &str, turn_id: &str, total_bytes: usize) -> RuntimeEvent {
        RuntimeEvent::ProviderInputObserved(ProviderInputObserved {
            strand_id: strand_id.to_string(),
            turn_id: turn_id.to_string(),
            round: 1,
            provider: "test".to_string(),
            model: "test-model".to_string(),
            input_items: 1,
            input_bytes: total_bytes,
            instructions_bytes: 0,
            reference_threshold_bytes: COMPACT_REMINDER_REFERENCE_BYTES,
            band: "soft".to_string(),
        })
    }

    #[test]
    fn provider_input_event_materializes_strand_local_santi_system_record() {
        let (service, _temp) = service();
        let strand = service.store.create_strand().expect("create strand");
        let turn_id = "turn_test";
        assert!(service.runtime_notices.publish(observed_event(
            &strand.id,
            turn_id,
            COMPACT_REMINDER_REFERENCE_BYTES + 1,
        )));

        service.drain_internal_runtime_notices_for_turn(turn_id);

        let messages = service.store.strand_messages(&strand.id).expect("messages");
        assert_eq!(messages.len(), 1);
        let message = &messages[0];
        assert_eq!(message.message.message_kind, MessageKind::SantiSystem);
        assert!(message.content_text.contains("kind: compact_reminder"));
        assert!(message.content_text.contains("scope: strand_local"));
        assert!(message.content_text.contains("wake: false"));
        assert!(message.content_text.contains("obligation: false"));
        assert!(
            message
                .content_text
                .contains("runtime did not compact or alter provider input")
        );
    }

    #[test]
    fn compact_reminder_record_does_not_wake_or_enter_request_inbox() {
        let (service, _temp) = service();
        let strand = service.store.create_strand().expect("create strand");
        let turn_id = "turn_test";
        service.runtime_notices.publish(observed_event(
            &strand.id,
            turn_id,
            COMPACT_REMINDER_REFERENCE_BYTES + 1,
        ));

        service.drain_internal_runtime_notices_for_turn(turn_id);

        let pending = service
            .store
            .strands_with_pending_requests()
            .expect("pending strands");
        assert!(pending.is_empty());
        let started = service
            .store
            .try_start_turn(&strand.id, "system", None)
            .expect("try start");
        assert!(started.is_none());
    }

    #[test]
    fn publishing_event_does_not_change_provider_input() {
        let (service, _temp) = service();
        let strand = service.store.create_strand().expect("create strand");
        service
            .store
            .append_santi_system_message(
                &strand.id,
                MessageContent::text("<system_message>\nkind: test\n</system_message>"),
                MessageIntake::Record,
            )
            .expect("append baseline");
        let before = format!(
            "{:?}",
            service
                .store
                .assembly_input(&strand.id)
                .expect("input before")
        );

        assert!(service.runtime_notices.publish(observed_event(
            &strand.id,
            "turn_test",
            COMPACT_REMINDER_REFERENCE_BYTES + 1,
        )));

        let after = format!(
            "{:?}",
            service
                .store
                .assembly_input(&strand.id)
                .expect("input after")
        );
        assert_eq!(before, after);
    }

    #[test]
    fn repeated_compact_reminder_events_are_coalesced_by_dedupe_key() {
        let (service, _temp) = service();
        let strand = service.store.create_strand().expect("create strand");
        let first = observed_event(&strand.id, "turn_a", COMPACT_REMINDER_REFERENCE_BYTES + 1);
        let duplicate = observed_event(&strand.id, "turn_a", COMPACT_REMINDER_REFERENCE_BYTES + 10);
        assert!(service.runtime_notices.publish(first));
        assert!(!service.runtime_notices.publish(duplicate));

        service.drain_internal_runtime_notices_for_turn("turn_a");

        let messages = service.store.strand_messages(&strand.id).expect("messages");
        assert_eq!(messages.len(), 1);
    }

    #[test]
    fn below_threshold_provider_input_event_is_silent() {
        let (service, _temp) = service();
        let strand = service.store.create_strand().expect("create strand");
        let turn_id = "turn_test";
        assert!(service.runtime_notices.publish(observed_event(
            &strand.id,
            turn_id,
            COMPACT_REMINDER_REFERENCE_BYTES - 1,
        )));

        service.drain_internal_runtime_notices_for_turn(turn_id);

        let messages = service.store.strand_messages(&strand.id).expect("messages");
        assert!(messages.is_empty());
    }

    #[test]
    fn observe_provider_input_publishes_without_touching_timeline() {
        let (service, _temp) = service();
        let strand = service.store.create_strand().expect("create strand");
        let before = service.store.strand_messages(&strand.id).expect("before");
        assert!(before.is_empty());

        let input = vec![ProviderItem::Message {
            role: "user".to_string(),
            content: "x".repeat(COMPACT_REMINDER_REFERENCE_BYTES + 1),
        }];
        service.observe_provider_input_for_notices(ProviderInputObservation {
            strand_id: &strand.id,
            turn_id: "turn_test",
            round: 1,
            provider: "test",
            model: "test-model",
            input: &input,
            instructions: None,
        });

        let after_publish = service
            .store
            .strand_messages(&strand.id)
            .expect("after publish");
        assert!(after_publish.is_empty());

        service.drain_internal_runtime_notices_for_turn("turn_test");
        let after_drain = service
            .store
            .strand_messages(&strand.id)
            .expect("after drain");
        assert_eq!(after_drain.len(), 1);
    }

    #[test]
    fn drain_for_turn_leaves_other_turn_events_queued() {
        let (service, _temp) = service();
        let strand = service.store.create_strand().expect("create strand");
        assert!(service.runtime_notices.publish(observed_event(
            &strand.id,
            "turn_a",
            COMPACT_REMINDER_REFERENCE_BYTES + 1,
        )));
        assert!(
            service
                .runtime_notices
                .publish(RuntimeEvent::ProviderInputObserved(ProviderInputObserved {
                    band: "higher".to_string(),
                    ..match observed_event(
                        &strand.id,
                        "turn_b",
                        COMPACT_REMINDER_REFERENCE_BYTES + 2
                    ) {
                        RuntimeEvent::ProviderInputObserved(event) => event,
                    }
                },))
        );

        service.drain_internal_runtime_notices_for_turn("turn_a");
        assert_eq!(service.store.strand_messages(&strand.id).unwrap().len(), 1);

        service.drain_internal_runtime_notices_for_turn("turn_b");
        assert_eq!(service.store.strand_messages(&strand.id).unwrap().len(), 2);
    }
}
