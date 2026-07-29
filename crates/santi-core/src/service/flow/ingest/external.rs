use super::*;

impl Service {
    pub async fn evented(
        &self,
        soul: &str,
        label: &str,
        system_text: String,
    ) -> Result<ingest::Outcome, String> {
        self.sourced(soul, label, system_text, None).await
    }

    pub async fn sourced(
        &self,
        soul: &str,
        label: &str,
        system_text: String,
        source: Option<ingest::Source>,
    ) -> Result<ingest::Outcome, String> {
        self.external(External {
            input: crate::service::Envelope {
                soul,
                label,
                text: system_text,
                source,
            },
            replay: None,
        })
        .await
    }

    pub async fn deliver(
        &self,
        input: crate::service::Envelope<'_>,
        delivery: crate::service::Delivery<'_>,
    ) -> Result<ingest::Outcome, String> {
        self.external(External {
            input,
            replay: Some(santi_estate::ReplayDraft::Webhook {
                subscription: delivery.subscription,
                delivery: delivery.id,
                digest: delivery.digest,
            }),
        })
        .await
    }

    pub(in crate::service) async fn external(
        &self,
        external: External<'_>,
    ) -> Result<ingest::Outcome, String> {
        let External { input, replay } = external;
        let strand = self
            .store
            .selected(
                &strand::Selector::ByLabel {
                    soul: input.soul.to_string(),
                    label: input.label.to_string(),
                },
                &crate::now(),
            )
            .await?;
        let (outcome, _driven) = self
            .enqueue(
                &strand,
                Ingest {
                    content: message::Content::text(input.text),
                    kind: message::Kind::SantiSystem,
                    trigger: "system",
                    source: input.source,
                    replay,
                },
            )
            .await?;
        Ok(outcome)
    }
}
