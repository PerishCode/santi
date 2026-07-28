use super::*;

impl Service {
    pub(super) async fn misfired(
        &self,
        strand: &str,
        turn: &str,
        error: &str,
        metadata: Metadata,
    ) -> (Option<Turn>, Fault) {
        let descriptor = crate::turn::Error::Provider.descriptor();
        match self
            .store
            .fail_classified(santi_estate::ClassifiedFailureDraft {
                turn,
                detail: error,
                incident: santi_error::Draft {
                    key: descriptor.key("strand", strand),
                    descriptor,
                    scope: santi_error::Scope::new("strand", strand),
                    source: santi_error::Source::new("santi-provider", metadata.stage.operation()),
                    message: "provider turn failed".to_string(),
                    context: serde_json::json!({
                        "schema": "santi.error.provider_turn.v1",
                        "turn": turn,
                        "provider": metadata.provider,
                        "model": metadata.model,
                        "stage": metadata.stage.name(),
                        "round": metadata.round,
                        "detail": error,
                    }),
                },
                occurred: &crate::now(),
            })
            .await
        {
            Ok(failure) => (Some(failure.turn), failure.fault),
            Err(persistence_error) => {
                eprintln!(
                    "santi: provider failure incident persistence failed for {turn}: {persistence_error}"
                );
                let held = self.store.fail_turn(turn, error, &crate::now()).await.ok();
                (held, unrecorded(strand, turn, persistence_error))
            }
        }
    }
}
