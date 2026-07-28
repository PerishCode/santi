use crate::service::Service;
use crate::{Fault, Ruled, engine, stream, turn::Turn};

impl Service {
    pub(super) async fn interrupted(
        &self,
        strand: &str,
        turn: &str,
        cause: crate::turn::Cause,
        error: &str,
    ) -> (Option<Turn>, Fault) {
        match self
            .store
            .interrupt_turn(santi_estate::InterruptionDraft {
                turn,
                cause,
                actor: crate::SYSTEM,
                occurred: &crate::now(),
            })
            .await
        {
            Ok(stopped) => {
                if let Some(marker) = stopped.notice {
                    self.publish(
                        strand,
                        stream::Payload::Message(crate::message::Beat::Created { message: marker }),
                    );
                }
                let fault = engine().transient(crate::Signal {
                    descriptor: crate::turn::Error::Interrupted.descriptor(),
                    source: santi_error::Source::new("santi-core", "turn_stop"),
                    scope: Some(santi_error::Scope::new("strand", strand)),
                    message: error.to_string(),
                    context: serde_json::json!({
                        "turn": turn,
                        "cause": cause.encode(),
                    }),
                });
                (Some(stopped.stop.turn), fault)
            }
            Err(detail) => {
                eprintln!("santi: interrupted turn persistence failed for {turn}: {detail}");
                (None, super::unwritten(strand, turn, detail))
            }
        }
    }
}
