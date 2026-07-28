use crate::Ruled;
use crate::service::flow::memory::{Gate, maintain};
use crate::service::{Service, drive};
use std::{future::Future, pin::Pin};

use super::*;
use crate::stream;

impl Service {
    pub(in crate::service) async fn poke(
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
            },
        )
        .await
    }

    pub(in crate::service) async fn poked(
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
            },
        )
        .await
    }

    fn poking<'a>(
        &'a self,
        strand: &'a str,
        drive: Drive<'a>,
    ) -> Pin<Box<dyn Future<Output = drive::Outcome> + Send + 'a>> {
        Box::pin(async move {
            if self.closing() {
                return drive::Outcome::Paused;
            }
            let strand = match self.store.strand(strand).await {
                Ok(Some(strand)) => strand,
                Ok(None) => {
                    return drive::Outcome::Failed(
                        self.stumbled(strand, drive, "strand not found".to_string())
                            .await,
                    );
                }
                Err(error) => {
                    return drive::Outcome::Failed(self.stumbled(strand, drive, error).await);
                }
            };
            match self.gate(&strand).await {
                Ok(Gate::Allow) => {}
                Ok(Gate::Pause {
                    maintenance_strand_id,
                }) => {
                    maintain(self, &maintenance_strand_id).await;
                    return drive::Outcome::Paused;
                }
                Err(error) => {
                    return drive::Outcome::Failed(self.stumbled(&strand.id, drive, error).await);
                }
            }
            if let Err(error) = self.absolve(&strand.id, "driver_remeasurement").await {
                return drive::Outcome::Failed(self.stumbled(&strand.id, drive, error).await);
            }
            let held = match self.admit_pending(&strand.id).await {
                Ok(held) => held,
                Err(error) => {
                    return drive::Outcome::Failed(self.stumbled(&strand.id, drive, error).await);
                }
            };
            if let Some(error) = held {
                return drive::Outcome::Held(error);
            }
            let turn_tag = crate::tag("turn");
            let trigger = match drive.trigger {
                "system" => crate::turn::Trigger::System,
                _ => crate::turn::Trigger::StrandSend,
            };
            let started = self
                .store
                .drain_turn(santi_estate::DrainDraft {
                    turn: &turn_tag,
                    strand: &strand.id,
                    trigger,
                    source: None,
                    actor: crate::SYSTEM,
                    created: &crate::now(),
                })
                .await;
            self.dispatched().await;
            match started {
                Ok(santi_estate::Opening::Started(started)) => {
                    let key = crate::drive::Error::Failed
                        .descriptor()
                        .key("strand", &strand.id);
                    let _ = self
                        .store
                        .resolve(
                            &key,
                            "strand.drive_started",
                            serde_json::json!({
                                "schema": "santi.error.strand_drive.resolution.v1",
                                "turn": started.turn.id,
                                "drained_count": started.drained.len(),
                            }),
                            &crate::now(),
                        )
                        .await;
                    self.refreshed().await;
                    let background = self.clone();
                    let strand = strand.id.clone();
                    let turn = started.turn.id.clone();
                    let control = background.register(&turn);
                    for message in started.drained.iter().cloned() {
                        self.publish(
                            &strand,
                            stream::Payload::Message(crate::message::Beat::Created { message }),
                        );
                    }
                    self.publish(
                        &strand,
                        stream::Payload::Turn(crate::turn::Beat::Started {
                            turn: started.turn.clone(),
                        }),
                    );
                    let context = {
                        let _entered = background.context.enter();
                        background.context.with(plumb::trace::Span::open("turn"))
                    };
                    tokio::spawn(context.carry(async move {
                        plumb::trace::Span::note("turn", &turn);
                        plumb::trace::Span::note("strand", &strand);
                        background.conduct(strand, turn, control).await;
                    }));
                    drive::Outcome::Started(started.turn, started.drained)
                }
                Ok(santi_estate::Opening::Running(turn)) => drive::Outcome::Running(turn),
                Ok(santi_estate::Opening::Idle) => drive::Outcome::Idle,
                Err(error) => drive::Outcome::Failed(self.stumbled(&strand.id, drive, error).await),
            }
        })
    }
}
