use std::time::Instant;

use serde_json::json;
use sha2::{Digest, Sha256};

use crate::store::{DEFAULT_SOUL_ID, Reservation};
use crate::{
    ErrorDescriptor, ErrorSource, IM_LABEL_PREFIX, IngestOutcome, MessageContent, MessageKind,
    SantiError, Signal, StrandSelector, WindowSendAccepted, WindowSendRequest, WindowTranscript,
    catalog, engine, prefixed_id, timestamp_now,
};

use super::Service;
use super::flow::Ingest;

const CONTENT_LIMIT: usize = 16 * 1024;
const CLIENT_KEY_LIMIT: usize = 128;
const UID_METADATA_LIMIT: usize = 128;
const RATE_BURST: f64 = 5.0;
const RATE_REFILL: f64 = 0.5;

pub enum Outcome {
    Accepted(WindowSendAccepted),
    Rejected(Box<SantiError>),
}

#[derive(Clone, Copy)]
pub(super) struct Pace {
    tokens: f64,
    refreshed: Instant,
}

pub fn window_participant(uid: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"authentik\0");
    hasher.update(uid.as_bytes());
    let digest = hasher.finalize();
    let hex: String = digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("window:{hex}")
}

fn content_hash(content: &str) -> String {
    let digest = Sha256::digest(content.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn rejection(
    descriptor: ErrorDescriptor,
    message: String,
    context: serde_json::Value,
) -> Box<SantiError> {
    Box::new(engine().transient(Signal {
        descriptor,
        source: ErrorSource::new("santi-core", "window_ingress"),
        scope: None,
        message,
        context,
    }))
}

impl Service {
    pub fn window_send(&self, uid: &str, request: WindowSendRequest) -> Result<Outcome, String> {
        if uid.trim().is_empty() {
            return Ok(Outcome::Rejected(rejection(
                catalog::WINDOW_IDENTITY_MISSING,
                "window identity header is missing or blank".to_string(),
                serde_json::Value::Null,
            )));
        }
        let client = request.client_message_id.as_str();
        if client.trim().is_empty() || client.len() > CLIENT_KEY_LIMIT {
            return Ok(Outcome::Rejected(rejection(
                catalog::WINDOW_CONTENT_INVALID,
                format!("client_message_id must be nonblank and at most {CLIENT_KEY_LIMIT} bytes"),
                serde_json::Value::Null,
            )));
        }
        let content = request.content.as_str();
        if content.trim().is_empty() {
            return Ok(Outcome::Rejected(rejection(
                catalog::WINDOW_CONTENT_INVALID,
                "content must contain text".to_string(),
                serde_json::Value::Null,
            )));
        }
        if content.len() > CONTENT_LIMIT {
            return Ok(Outcome::Rejected(rejection(
                catalog::WINDOW_CONTENT_OVERSIZE,
                format!("content exceeds the {CONTENT_LIMIT} byte limit"),
                json!({ "limit_bytes": CONTENT_LIMIT, "content_bytes": content.len() }),
            )));
        }

        let participant = window_participant(uid);
        let hash = content_hash(content);
        if let Some(reserved) = self.store.window_message(&participant, client)? {
            if reserved.content_hash == hash {
                return Ok(Outcome::Accepted(WindowSendAccepted {
                    status: "accepted".to_string(),
                    message_id: reserved.message_id,
                    client_message_id: client.to_string(),
                    cursor: reserved.cursor,
                    received_at: reserved.received_at,
                    receipt_id: reserved.inbox_id,
                }));
            }
            return Ok(Outcome::Rejected(rejection(
                catalog::WINDOW_MESSAGE_CONFLICT,
                "client_message_id was already used with different content".to_string(),
                serde_json::Value::Null,
            )));
        }

        if let Some(retry_after) = self.window_rate_denied(&participant) {
            return Ok(Outcome::Rejected(rejection(
                catalog::WINDOW_RATE_LIMITED,
                "window speech is rate limited; retry later".to_string(),
                json!({ "retry_after_seconds": retry_after }),
            )));
        }

        self.store.ensure_im_participant(&participant, "human")?;
        let message_id = prefixed_id("msg");
        let received_at = timestamp_now();
        let mut bounded_uid = uid.to_string();
        bounded_uid.truncate(UID_METADATA_LIMIT);
        let outcome = self.accept(
            StrandSelector::ByLabel {
                soul_id: DEFAULT_SOUL_ID.to_string(),
                label: format!("{IM_LABEL_PREFIX}{participant}"),
            },
            Ingest {
                content: MessageContent::text(content.to_string()),
                kind: MessageKind::Text,
                trigger: "strand_send",
                source: Some(
                    crate::InboxSource::new("window")
                        .with_ref(participant.clone())
                        .with_metadata(json!({ "uid": bounded_uid })),
                ),
                window: Some(Reservation {
                    participant: &participant,
                    client,
                    message: &message_id,
                    hash: &hash,
                    received: &received_at,
                }),
            },
        )?;
        match outcome {
            IngestOutcome::Accepted { receipt } => Ok(Outcome::Accepted(WindowSendAccepted {
                status: "accepted".to_string(),
                message_id,
                client_message_id: client.to_string(),
                cursor: None,
                received_at,
                receipt_id: receipt.inbox_id,
            })),
            IngestOutcome::Rejected { error } => Ok(Outcome::Rejected(error)),
        }
    }

    pub fn window_transcript(
        &self,
        uid: &str,
        since: i64,
        limit: usize,
    ) -> Result<WindowTranscript, String> {
        let participant = window_participant(uid);
        let bounded = limit.clamp(1, 200);
        let label = format!("{IM_LABEL_PREFIX}{participant}");
        let Some(strand) = self.store.labeled_strand(DEFAULT_SOUL_ID, &label)? else {
            return Ok(WindowTranscript {
                participant,
                entries: Vec::new(),
                next_since: since,
                has_more: false,
                empty: true,
            });
        };
        let (entries, has_more, empty) =
            self.store.window_transcript(&strand.id, since, bounded)?;
        let next_since = entries.last().map(|entry| entry.seq).unwrap_or(since);
        Ok(WindowTranscript {
            participant,
            entries,
            next_since,
            has_more,
            empty,
        })
    }

    fn window_rate_denied(&self, participant: &str) -> Option<u64> {
        let mut rates = self.window_rates.lock().unwrap();
        let now = Instant::now();
        let pace = rates.get(participant).copied().unwrap_or(Pace {
            tokens: RATE_BURST,
            refreshed: now,
        });
        let refilled = (pace.tokens
            + now.duration_since(pace.refreshed).as_secs_f64() * RATE_REFILL)
            .min(RATE_BURST);
        if refilled >= 1.0 {
            rates.insert(
                participant.to_string(),
                Pace {
                    tokens: refilled - 1.0,
                    refreshed: now,
                },
            );
            None
        } else {
            rates.insert(
                participant.to_string(),
                Pace {
                    tokens: refilled,
                    refreshed: now,
                },
            );
            Some(((1.0 - refilled) / RATE_REFILL).ceil() as u64)
        }
    }
}
