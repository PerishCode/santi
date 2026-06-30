use std::{
    collections::{HashMap, HashSet},
    time::Instant,
};

use santi_provider::{ProviderEvent, ProviderStreamTrace};

pub(super) struct ProviderTurnTiming<'a> {
    turn_id: &'a str,
    turn_started: Instant,
    round_started: Option<Instant>,
    response_started: Option<Instant>,
    chunks: usize,
    bytes: usize,
    raw_events: usize,
    raw_counts: HashMap<String, usize>,
    seen_raw_types: HashSet<String>,
}

impl<'a> ProviderTurnTiming<'a> {
    pub(super) fn new(turn_id: &'a str) -> Self {
        let timing = Self {
            turn_id,
            turn_started: Instant::now(),
            round_started: None,
            response_started: None,
            chunks: 0,
            bytes: 0,
            raw_events: 0,
            raw_counts: HashMap::new(),
            seen_raw_types: HashSet::new(),
        };
        timing.log("turn_started", 0, "");
        timing
    }

    pub(super) fn request_built(
        &mut self,
        round: usize,
        input_len: usize,
        instructions_len: usize,
    ) {
        self.round_started = Some(Instant::now());
        self.response_started = None;
        self.log(
            "request_built",
            round,
            &format!("input_len={input_len} instructions_len={instructions_len}"),
        );
    }

    pub(super) fn http_response_started(&mut self, round: usize) {
        self.response_started = Some(Instant::now());
        self.log("http_response_started", round, &self.round_elapsed());
    }

    pub(super) fn first_sse_event(&self, round: usize, event_name: &'static str) {
        self.log(
            "first_sse_event",
            round,
            &format!("event={event_name} {}", self.response_elapsed()),
        );
    }

    pub(super) fn first_text_delta(&self, round: usize) {
        self.log("first_text_delta", round, &self.response_elapsed());
    }

    pub(super) fn function_call_requested(&self, round: usize, name: &str) {
        self.log(
            "function_call_requested",
            round,
            &format!("name={name} {}", self.response_elapsed()),
        );
    }

    pub(super) fn completed(&self, round: usize) {
        self.log(
            "provider_completed",
            round,
            &format!(
                "{} chunks={} bytes={} raw_events={}",
                self.response_elapsed(),
                self.chunks,
                self.bytes,
                self.raw_events
            ),
        );
    }

    pub(super) fn tool_outputs_started(&self, round: usize, count: usize) {
        self.log("tool_outputs_started", round, &format!("count={count}"));
    }

    pub(super) fn tool_outputs_completed(&self, round: usize, count: usize) {
        self.log("tool_outputs_completed", round, &format!("count={count}"));
    }

    pub(super) fn failed(&self, round: usize, stage: &str, error: &str) {
        self.log(
            "failed",
            round,
            &format!(
                "stage={stage} chunks={} bytes={} raw_events={} error={error}",
                self.chunks, self.bytes, self.raw_events
            ),
        );
    }

    pub(super) fn provider_trace(&mut self, round: usize, trace: ProviderStreamTrace) {
        match trace {
            ProviderStreamTrace::Chunk { bytes } => {
                self.chunks += 1;
                self.bytes += bytes;
                if self.chunks == 1 {
                    self.log(
                        "provider_chunk",
                        round,
                        &format!("chunk_bytes={bytes} total_bytes={}", self.bytes),
                    );
                }
            }
            ProviderStreamTrace::RawEvent {
                raw_type,
                mapped_events,
            } => {
                self.raw_events += 1;
                let (count, is_first) = self.record_raw_event(&raw_type);
                if should_log_raw_event(is_first, &mapped_events) {
                    self.log(
                        "provider_raw_event",
                        round,
                        &format!(
                            "raw_type={raw_type} raw_count={count} mapped={mapped} raw_events={}",
                            self.raw_events,
                            mapped = mapped_event_list(&mapped_events)
                        ),
                    );
                }
            }
        }
    }

    fn record_raw_event(&mut self, raw_type: &str) -> (usize, bool) {
        let count = {
            let count = self.raw_counts.entry(raw_type.to_string()).or_insert(0);
            *count += 1;
            *count
        };
        let is_first = self.seen_raw_types.insert(raw_type.to_string());
        (count, is_first)
    }

    fn round_elapsed(&self) -> String {
        self.round_started
            .map(|started| format!("round_ms={}", started.elapsed().as_millis()))
            .unwrap_or_default()
    }

    fn response_elapsed(&self) -> String {
        self.response_started
            .map(|started| format!("response_ms={}", started.elapsed().as_millis()))
            .unwrap_or_default()
    }

    fn log(&self, event: &str, round: usize, fields: &str) {
        eprintln!(
            "santi-timing turn_id={} event={} round={} turn_ms={} {}",
            self.turn_id,
            event,
            round,
            self.turn_started.elapsed().as_millis(),
            fields
        );
    }
}

fn should_log_raw_event(is_first: bool, mapped_events: &[String]) -> bool {
    is_first
        || mapped_events
            .iter()
            .any(|event| !matches!(event.as_str(), "reasoning_summary_delta" | "text_delta"))
}

fn mapped_event_list(mapped_events: &[String]) -> String {
    if mapped_events.is_empty() {
        "none".to_string()
    } else {
        mapped_events.join(",")
    }
}

pub(super) fn provider_event_name(event: &ProviderEvent) -> &'static str {
    match event {
        ProviderEvent::ResponseStarted { .. } => "response_started",
        ProviderEvent::ResponseInProgress { .. } => "response_in_progress",
        ProviderEvent::ReasoningSummaryDelta(_) => "reasoning_summary_delta",
        ProviderEvent::ReasoningSummaryDone(_) => "reasoning_summary_done",
        ProviderEvent::TextDelta(_) => "text_delta",
        ProviderEvent::FunctionCallRequested(_) => "function_call_requested",
        ProviderEvent::Completed { .. } => "completed",
        ProviderEvent::Failed(_) => "failed",
        ProviderEvent::StreamTrace(_) => "stream_trace",
    }
}
