use crate::service::flow::memory::{Gate, maintain};
use crate::service::{Service, drive};
use crate::store::{Launch, Opened, errors::drive::Input};
use crate::{Fault, catalog, engine};

use super::*;
use crate::{ingest, message, strand, stream};

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

    pub fn drive(&self, strand: &str) -> Result<crate::drive::Response, Box<Fault>> {
        self.store
            .strand(strand)
            .map_err(|message| Box::new(erred(catalog::INTERNAL, strand, message)))?
            .ok_or_else(|| {
                Box::new(erred(
                    catalog::NOT_FOUND,
                    strand,
                    "strand not found".to_string(),
                ))
            })?;
        match self.poked(strand, "strand_send", None, "operator_redrive") {
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

    pub(in crate::service) fn poke(
        &self,
        strand: &str,
        trigger: &str,
        inbox: Option<&str>,
        operation: &str,
    ) -> drive::Outcome {
        self.poking(
            strand,
            Drive {
                trigger,
                inbox,
                operation,
                recovered: false,
            },
        )
    }

    pub(in crate::service) fn poked(
        &self,
        strand: &str,
        trigger: &str,
        inbox: Option<&str>,
        operation: &str,
    ) -> drive::Outcome {
        self.poking(
            strand,
            Drive {
                trigger,
                inbox,
                operation,
                recovered: true,
            },
        )
    }

    fn poking(&self, strand: &str, drive: Drive<'_>) -> drive::Outcome {
        if self.closing() {
            return drive::Outcome::Paused;
        }
        let strand = match self.store.strand(strand) {
            Ok(Some(strand)) => strand,
            Ok(None) => {
                return drive::Outcome::Failed(self.stumbled(
                    strand,
                    drive,
                    "strand not found".to_string(),
                ));
            }
            Err(error) => {
                return drive::Outcome::Failed(self.stumbled(strand, drive, error));
            }
        };
        match self.gate(&strand) {
            Ok(Gate::Allow) => {}
            Ok(Gate::Pause {
                maintenance_strand_id,
            }) => {
                maintain(self, &maintenance_strand_id);
                return drive::Outcome::Paused;
            }
            Err(error) => {
                return drive::Outcome::Failed(self.stumbled(&strand.id, drive, error));
            }
        }
        if let Err(error) = self.absolve(&strand.id, "driver_remeasurement") {
            return drive::Outcome::Failed(self.stumbled(&strand.id, drive, error));
        }
        let admission = match self.admission(&strand.id) {
            Ok(admission) => admission,
            Err(error) => {
                return drive::Outcome::Failed(self.stumbled(&strand.id, drive, error));
            }
        };
        let started = self.store.budgeted(Launch {
            strand: &strand.id,
            trigger: drive.trigger,
            reference: None,
            admission: admission.as_ref(),
            recover: drive.recovered,
        });
        self.dispatched();
        match started {
            Ok(Opened::Started(started)) => {
                self.refreshed();
                for message in started.drained.iter().cloned() {
                    self.publish(&strand.id, stream::Payload::MessageCreated { message });
                }
                self.publish(
                    &strand.id,
                    stream::Payload::TurnStarted {
                        turn: started.turn.clone(),
                    },
                );
                let background = self.clone();
                let strand = strand.id.clone();
                let turn = started.turn.id.clone();
                tokio::spawn(async move {
                    background.conduct(strand, turn).await;
                });
                drive::Outcome::Started(started.turn, started.drained)
            }
            Ok(Opened::Running(turn)) => drive::Outcome::Running(turn),
            Ok(Opened::Idle) => drive::Outcome::Idle,
            Ok(Opened::Held(error)) => drive::Outcome::Held(error),
            Err(error) => drive::Outcome::Failed(self.stumbled(&strand.id, drive, error)),
        }
    }

    fn stumbled(&self, strand: &str, drive: Drive<'_>, detail: String) -> Fault {
        self.degrade();
        let error = match self.store.stumbled(
            strand,
            Input {
                operation: drive.operation,
                trigger: drive.trigger,
                inbox: drive.inbox,
                detail: &detail,
            },
        ) {
            Ok(error) => error,
            Err(persistence_error) => engine().transient(crate::Signal {
                descriptor: catalog::ERROR_ENGINE_PERSISTENCE_FAILED,
                source: santi_error::Source::new("santi-core", "strand_drive_failure"),
                scope: Some(santi_error::Scope::new("strand", strand)),
                message: "failed to persist strand driver incident".to_string(),
                context: serde_json::json!({
                    "accepted_before_failure": drive.inbox.is_some(),
                    "inbox": drive.inbox,
                    "detail": persistence_error,
                }),
            }),
        };
        eprintln!(
            "santi: strand drive failed code={} incident_id={} strand={} operation={} accepted_before_failure={}",
            error.code,
            error.incident.as_deref().unwrap_or("-"),
            strand,
            drive.operation,
            drive.inbox.is_some(),
        );
        self.dispatched();
        error
    }
}
