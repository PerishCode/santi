use super::tool::{self, CallDraft};
use super::{Store, read};
use keel::{Op, Rank, form};
use santi_model::effect;

mod projection;
mod types;
mod write;

pub use types::{EffectDraft, RedemptionDraft};
use write::shift;

impl Store {
    pub async fn prepare_invocation(
        &self,
        call: CallDraft<'_>,
        effect: Option<EffectDraft<'_>>,
    ) -> Result<(santi_model::tool::Call, Option<effect::Effect>), String> {
        let arguments = serde_json::to_string(call.arguments).map_err(|error| error.to_string())?;
        let metadata = effect
            .as_ref()
            .and_then(|effect| effect.metadata)
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| error.to_string())?;
        let call_tag = call.tag.to_string();
        let effect_tag = effect.as_ref().map(|effect| effect.tag.to_string());
        self.core
            .batch(async |tx| {
                tool::put_call(tx, call, &arguments).await?;
                if let Some(effect) = effect {
                    write::prepare(tx, effect, metadata.as_deref()).await?;
                }
                Ok(())
            })
            .await
            .map_err(read::error)?;
        let call = self
            .call(&call_tag)
            .await?
            .ok_or_else(|| "prepared invocation call missing".to_string())?;
        let effect = match effect_tag {
            Some(tag) => Some(
                self.effect(&tag)
                    .await?
                    .map(|status| status.effect)
                    .ok_or_else(|| "prepared invocation effect missing".to_string())?,
            ),
            None => None,
        };
        Ok((call, effect))
    }

    pub async fn prepare_effect(&self, draft: EffectDraft<'_>) -> Result<effect::Effect, String> {
        let metadata = draft
            .metadata
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| error.to_string())?;
        self.core
            .batch(async |tx| write::prepare(tx, draft, metadata.as_deref()).await)
            .await
            .map_err(read::error)?;
        self.effect(draft.tag)
            .await?
            .map(|status| status.effect)
            .ok_or_else(|| "prepared effect missing".to_string())
    }

    pub async fn effect(&self, tag: &str) -> Result<Option<effect::Status>, String> {
        projection::status(self, tag).await
    }

    pub async fn dispatch_effect(
        &self,
        tag: &str,
        occurred: &str,
    ) -> Result<effect::Effect, String> {
        self.move_effect(
            tag,
            &["prepared"],
            effect::State::Dispatching,
            None,
            occurred,
        )
        .await
    }

    pub async fn unknown_effect(
        &self,
        tag: &str,
        evidence: &str,
        occurred: &str,
    ) -> Result<effect::Effect, String> {
        self.move_effect(
            tag,
            &["dispatching"],
            effect::State::Unknown,
            Some(evidence),
            occurred,
        )
        .await
    }

    pub async fn settle_effect(
        &self,
        tag: &str,
        outcome: effect::Outcome,
        evidence: &str,
        occurred: &str,
    ) -> Result<Option<effect::Status>, String> {
        if evidence.trim().is_empty() {
            return Err("effect resolution evidence must not be empty".to_string());
        }
        if self.effect(tag).await?.is_none() {
            return Ok(None);
        }
        self.move_effect(
            tag,
            &["unknown"],
            effect::State::Settled(outcome),
            None,
            occurred,
        )
        .await?;
        self.effect(tag).await
    }

    pub async fn redeem_effect(
        &self,
        tag: &str,
        draft: RedemptionDraft<'_>,
    ) -> Result<santi_model::tool::Reply, String> {
        let output = draft
            .output
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| error.to_string())?;
        let result = draft.result.to_string();
        self.core
            .batch(async |tx| write::redeem(tx, tag, draft, output.as_deref()).await)
            .await
            .map_err(read::error)?;
        self.reply(&result)
            .await?
            .ok_or_else(|| "created effect tool result missing".to_string())
    }

    pub async fn reconcile_effects(&self, turn: &str, occurred: &str) -> Result<(), String> {
        self.core
            .batch(async |tx| reconcile_in(tx, turn, occurred).await)
            .await
            .map_err(read::error)
    }

    async fn move_effect(
        &self,
        tag: &str,
        expected: &[&str],
        state: effect::State,
        error: Option<&str>,
        occurred: &str,
    ) -> Result<effect::Effect, String> {
        self.core
            .batch(async |tx| {
                shift(tx, tag, expected, state, error, occurred)
                    .await
                    .map(|_| ())
            })
            .await
            .map_err(read::error)?;
        self.effect(tag)
            .await?
            .map(|status| status.effect)
            .ok_or_else(|| "shifted effect missing".to_string())
    }
}

pub(in crate::store) async fn reconcile_in(
    tx: &mut keel::Tx<'_, keel::adapt::db::Sqlite>,
    turn: &str,
    occurred: &str,
) -> Result<(), keel::adapt::Error> {
    let turn = write::relation(tx, "Turn", turn).await?;
    let effects = tx
        .ask(
            &form("StrandEffect")
                .when("turn", Op::Eq, &turn.key().to_string())
                .order("created", Rank::Asc)
                .order("tag", Rank::Asc),
        )
        .await?;
    for effect in effects.rows() {
        let tag = effect
            .text("tag")
            .ok_or_else(|| keel::adapt::Error::Adapt("effect tag missing".into()))?;
        reconcile(tx, tag, effect.text("state"), occurred).await?;
    }
    Ok(())
}

async fn reconcile(
    tx: &mut keel::Tx<'_, keel::adapt::db::Sqlite>,
    tag: &str,
    state: Option<&str>,
    occurred: &str,
) -> Result<(), keel::adapt::Error> {
    match state {
        Some("prepared") => {
            shift(
                tx,
                tag,
                &["prepared"],
                effect::State::Settled(effect::Outcome::NotApplied),
                None,
                occurred,
            )
            .await?;
        }
        Some("dispatching") => {
            shift(
                tx,
                tag,
                &["dispatching"],
                effect::State::Unknown,
                None,
                occurred,
            )
            .await?;
        }
        _ => {}
    }
    Ok(())
}
