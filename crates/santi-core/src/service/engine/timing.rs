use std::{
    collections::{HashMap, HashSet},
    time::Instant,
};

use santi_provider::{Event, Trace};

pub(in crate::service) struct Turn<'a> {
    turn: &'a str,
    begun: Instant,
    opened: Option<Instant>,
    started: Option<Instant>,
    chunks: usize,
    bytes: usize,
    raws: usize,
    counts: HashMap<String, usize>,
    seen: HashSet<String>,
}

impl<'a> Turn<'a> {
    pub(in crate::service) fn new(turn: &'a str) -> Self {
        let timing = Self {
            turn,
            begun: Instant::now(),
            opened: None,
            started: None,
            chunks: 0,
            bytes: 0,
            raws: 0,
            counts: HashMap::new(),
            seen: HashSet::new(),
        };
        timing.log("turn_started", 0, "");
        timing
    }

    pub(in crate::service) fn built(
        &mut self,
        round: usize,
        input_len: usize,
        instructions_len: usize,
    ) {
        self.opened = Some(Instant::now());
        self.started = None;
        self.log(
            "request_built",
            round,
            &format!("input_len={input_len} instructions_len={instructions_len}"),
        );
    }

    pub(in crate::service) fn reached(&mut self, round: usize) {
        self.started = Some(Instant::now());
        self.log("http_response_started", round, &self.rounded());
    }

    pub(in crate::service) fn first(&self, round: usize, event_name: &'static str) {
        self.log(
            "first_sse_event",
            round,
            &format!("event={event_name} {}", self.lapsed()),
        );
    }

    pub(in crate::service) fn uttered(&self, round: usize) {
        self.log("first_text_delta", round, &self.lapsed());
    }

    pub(in crate::service) fn called(&self, round: usize, name: &str) {
        self.log(
            "function_call_requested",
            round,
            &format!("name={name} {}", self.lapsed()),
        );
    }

    pub(in crate::service) fn completed(&self, round: usize) {
        self.log(
            "provider_completed",
            round,
            &format!(
                "{} chunks={} bytes={} raw_events={}",
                self.lapsed(),
                self.chunks,
                self.bytes,
                self.raws
            ),
        );
    }

    pub(in crate::service) fn outputting(&self, round: usize, count: usize) {
        self.log("tool_outputs_started", round, &format!("count={count}"));
    }

    pub(in crate::service) fn outputted(&self, round: usize, count: usize) {
        self.log("tool_outputs_completed", round, &format!("count={count}"));
    }

    pub(in crate::service) fn failed(&self, round: usize, stage: &str, error: &str) {
        self.log(
            "failed",
            round,
            &format!(
                "stage={stage} chunks={} bytes={} raw_events={} error={error}",
                self.chunks, self.bytes, self.raws
            ),
        );
    }

    pub(in crate::service) fn traced(&mut self, round: usize, trace: Trace) {
        match trace {
            Trace::Chunk { bytes } => {
                self.chunks += 1;
                self.bytes += bytes;
                if self.chunks == 1 {
                    self.log(
                        "provider_chunk",
                        round,
                        &format!("chunk_bytes={bytes} total={}", self.bytes),
                    );
                }
            }
            Trace::Raw { kind, mapped } => {
                self.raws += 1;
                let (count, first) = self.tallied(&kind);
                if loggable(first, &mapped) {
                    self.log(
                        "provider_raw_event",
                        round,
                        &format!(
                            "raw_type={kind} raw_count={count} mapped={mapped} raw_events={}",
                            self.raws,
                            mapped = listed(&mapped)
                        ),
                    );
                }
            }
        }
    }

    fn tallied(&mut self, kind: &str) -> (usize, bool) {
        let count = {
            let count = self.counts.entry(kind.to_string()).or_insert(0);
            *count += 1;
            *count
        };
        let first = self.seen.insert(kind.to_string());
        (count, first)
    }

    fn rounded(&self) -> String {
        self.opened
            .map(|started| format!("round_ms={}", started.elapsed().as_millis()))
            .unwrap_or_default()
    }

    fn lapsed(&self) -> String {
        self.started
            .map(|started| format!("response_ms={}", started.elapsed().as_millis()))
            .unwrap_or_default()
    }

    fn log(&self, event: &str, round: usize, fields: &str) {
        eprintln!(
            "santi-timing turn={} event={} round={} turn_ms={} {}",
            self.turn,
            event,
            round,
            self.begun.elapsed().as_millis(),
            fields
        );
    }
}

fn loggable(first: bool, mapped: &[String]) -> bool {
    first
        || mapped
            .iter()
            .any(|event| !matches!(event.as_str(), "reasoning_summary_delta" | "text_delta"))
}

fn listed(mapped: &[String]) -> String {
    if mapped.is_empty() {
        "none".to_string()
    } else {
        mapped.join(",")
    }
}

pub(in crate::service) fn named(event: &Event) -> &'static str {
    match event {
        Event::Started { .. } => "response_started",
        Event::Working { .. } => "response_in_progress",
        Event::Thinking(_) => "reasoning_summary_delta",
        Event::Thought(_) => "reasoning_summary_done",
        Event::Text(_) => "text_delta",
        Event::Called(_) => "function_call_requested",
        Event::Completed { .. } => "completed",
        Event::Failed(_) => "failed",
        Event::Traced(_) => "stream_trace",
    }
}
