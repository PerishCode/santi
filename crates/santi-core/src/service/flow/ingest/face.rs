use crate::service::{Service, drive};
use crate::{Fault, catalog, ingest, message, strand};

use super::{Ingest, erred};

impl Service {
    pub async fn send(&self, strand: &str, request: strand::Post) -> Result<strand::Posted, Fault> {
        let text = request.text();
        if text.trim().is_empty() {
            return Err(erred(
                catalog::INVALID_ARGUMENT,
                strand,
                "send content must contain text".to_string(),
            ));
        }
        let strand = self
            .store
            .strand(strand)
            .await
            .map_err(|message| erred(catalog::INTERNAL, strand, message))?
            .ok_or_else(|| erred(catalog::NOT_FOUND, strand, "strand not found".to_string()))?;

        let (outcome, drive) = self
            .enqueue(
                &strand,
                Ingest {
                    content: message::Content {
                        parts: request.content,
                    },
                    kind: message::Kind::Text,
                    trigger: "strand_send",
                    source: Some(ingest::Source::new("strand_send").with_ref(strand.id.clone())),
                    replay: None,
                },
            )
            .await
            .map_err(|message| erred(catalog::INTERNAL, &strand.id, message))?;
        let receipt = match outcome {
            ingest::Outcome::Accepted { receipt } => receipt,
            ingest::Outcome::Rejected { error } => return Err(*error),
        };
        let (turn, message) = match drive {
            drive::Outcome::Started(turn, mut drained) => (Some(turn), drained.pop()),
            drive::Outcome::Running(turn) => (Some(turn), None),
            drive::Outcome::Idle
            | drive::Outcome::Held(_)
            | drive::Outcome::Paused
            | drive::Outcome::Failed(_) => (None, None),
        };

        Ok(strand::Posted {
            strand,
            receipt,
            turn,
            message,
        })
    }

    pub async fn drive(&self, strand: &str) -> Result<crate::drive::Response, Box<Fault>> {
        self.store
            .strand(strand)
            .await
            .map_err(|message| Box::new(erred(catalog::INTERNAL, strand, message)))?
            .ok_or_else(|| {
                Box::new(erred(
                    catalog::NOT_FOUND,
                    strand,
                    "strand not found".to_string(),
                ))
            })?;
        match self
            .poked(strand, "strand_send", None, "operator_redrive")
            .await
        {
            drive::Outcome::Started(turn, _) => Ok(crate::drive::Response {
                strand: strand.to_string(),
                state: crate::drive::State::Started,
                turn: Some(turn),
            }),
            drive::Outcome::Running(turn) => Ok(crate::drive::Response {
                strand: strand.to_string(),
                state: crate::drive::State::Running,
                turn: Some(turn),
            }),
            drive::Outcome::Idle => Ok(crate::drive::Response {
                strand: strand.to_string(),
                state: crate::drive::State::Idle,
                turn: None,
            }),
            drive::Outcome::Paused => Ok(crate::drive::Response {
                strand: strand.to_string(),
                state: crate::drive::State::Paused,
                turn: None,
            }),
            drive::Outcome::Held(error) | drive::Outcome::Failed(error) => Err(Box::new(error)),
        }
    }
}
