use super::*;

impl Service {
    pub(super) async fn tripped(
        &self,
        strand: &str,
        turn: &str,
        error: &str,
        operation: Operation,
    ) -> (Option<Turn>, Fault) {
        let descriptor = crate::turn::Error::Runtime.descriptor();
        match self
            .store
            .fail_classified(santi_estate::ClassifiedFailureDraft {
                turn,
                detail: error,
                incident: santi_error::Draft {
                    key: descriptor.key("strand", strand),
                    descriptor,
                    scope: santi_error::Scope::new("strand", strand),
                    source: santi_error::Source::new("santi-core", operation.name()),
                    message: "turn failed inside the runtime".to_string(),
                    context: serde_json::json!({
                        "schema": "santi.error.runtime_turn.v1",
                        "turn": turn,
                        "operation": operation.name(),
                        "detail": error,
                        "trace": format!("log://turn/{turn}"),
                    }),
                },
                occurred: &crate::now(),
            })
            .await
        {
            Ok(failure) => (Some(failure.turn), failure.fault),
            Err(persistence_error) => {
                eprintln!(
                    "santi: runtime turn failure persistence failed for {turn}: {persistence_error}"
                );
                (None, unwritten(strand, turn, persistence_error))
            }
        }
    }
}
