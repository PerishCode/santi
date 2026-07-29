use super::{EffectDraft, RedemptionDraft};
use crate::store::tool::{ReplyDraft, put_reply};
use keel::adapt::db::Sqlite;
use keel::{Op, Tx, form};
use santi_model::effect;

pub(super) struct Writer<'a, 'tx> {
    tx: &'a mut Tx<'tx, Sqlite>,
}

pub(super) struct Shift<'a> {
    pub tag: &'a str,
    pub expected: &'a [&'a str],
    pub state: effect::State,
    pub error: Option<&'a str>,
    pub occurred: &'a str,
}

impl<'a, 'tx> Writer<'a, 'tx> {
    pub fn new(tx: &'a mut Tx<'tx, Sqlite>) -> Self {
        Self { tx }
    }

    pub async fn prepare(
        &mut self,
        draft: EffectDraft<'_>,
        metadata: Option<&str>,
    ) -> Result<(), keel::adapt::Error> {
        let turn = self.relation("Turn", draft.turn).await?;
        let call = self.call(draft.call, turn.key()).await?;
        let turn = turn.key().to_string();
        let mut fields = vec![
            ("tag", draft.tag),
            ("effect_type", draft.kind),
            ("created", draft.created),
            ("updated", draft.created),
            ("turn", turn.as_str()),
        ];
        if let Some(call) = call.as_deref() {
            fields.push(("call", call));
        }
        if let Some(metadata) = metadata {
            fields.push(("metadata", metadata));
        }
        self.tx.put("StrandEffect", &fields).await?;
        Ok(())
    }

    pub async fn shift(&mut self, draft: Shift<'_>) -> Result<i64, keel::adapt::Error> {
        let row = self
            .tx
            .one(&form("StrandEffect").when("tag", Op::Eq, draft.tag))
            .await?
            .ok_or_else(|| keel::adapt::Error::Missing(draft.tag.into()))?;
        let current = row
            .text("state")
            .ok_or_else(|| keel::adapt::Error::Adapt("effect state missing".into()))?;
        if !draft.expected.contains(&current) {
            return Err(keel::adapt::Error::Adapt(format!(
                "effect state {current} cannot transition to {}",
                state_text(&draft.state)
            )));
        }
        let mut fields = vec![
            ("state", state_text(&draft.state)),
            ("updated", draft.occurred),
        ];
        match draft.state {
            effect::State::Dispatching => fields.push(("dispatched", draft.occurred)),
            effect::State::Settled(_) => fields.push(("settled", draft.occurred)),
            effect::State::Prepared | effect::State::Unknown => {}
        }
        if let Some(error) = draft.error {
            fields.push(("error", error));
        }
        self.tx.set("StrandEffect", row.key(), &fields).await?;
        Ok(row.key())
    }

    pub async fn redeem(
        &mut self,
        tag: &str,
        draft: RedemptionDraft<'_>,
        output: Option<&str>,
    ) -> Result<(), keel::adapt::Error> {
        let row = self
            .tx
            .one(&form("StrandEffect").when("tag", Op::Eq, tag))
            .await?
            .ok_or_else(|| keel::adapt::Error::Missing(tag.into()))?;
        let call = self.relation("ToolCall", draft.call).await?;
        if row.int("call") != Some(call.key()) {
            return Err(keel::adapt::Error::Adapt(
                "effect belongs to another tool call".into(),
            ));
        }
        let expected = match &draft.outcome {
            effect::Outcome::Applied => &["dispatching"][..],
            effect::Outcome::NotApplied => &["prepared", "dispatching"][..],
        };
        let state = effect::State::Settled(draft.outcome.clone());
        let current = row
            .text("state")
            .ok_or_else(|| keel::adapt::Error::Adapt("effect state missing".into()))?;
        if !expected.contains(&current) {
            return Err(keel::adapt::Error::Adapt(format!(
                "effect state {current} cannot transition to {}",
                state_text(&state)
            )));
        }
        put_reply(
            self.tx,
            ReplyDraft {
                tag: draft.result,
                call: draft.call,
                output: draft.output,
                error: draft.error,
                created: draft.occurred,
            },
            output,
        )
        .await?;
        let key = self
            .shift(Shift {
                tag,
                expected,
                state,
                error: draft.error,
                occurred: draft.occurred,
            })
            .await?;
        self.tx
            .set("StrandEffect", key, &[("result", draft.result)])
            .await?;
        Ok(())
    }

    pub async fn relation(
        &mut self,
        unit: &str,
        tag: &str,
    ) -> Result<keel::Row, keel::adapt::Error> {
        self.tx
            .one(&form(unit).when("tag", Op::Eq, tag))
            .await?
            .ok_or_else(|| keel::adapt::Error::Missing(format!("{unit} {tag}")))
    }

    async fn call(
        &mut self,
        tag: Option<&str>,
        turn: i64,
    ) -> Result<Option<String>, keel::adapt::Error> {
        let Some(tag) = tag else {
            return Ok(None);
        };
        let call = self.relation("ToolCall", tag).await?;
        if call.int("turn") != Some(turn) {
            return Err(keel::adapt::Error::Adapt(
                "effect call belongs to another turn".into(),
            ));
        }
        Ok(Some(call.key().to_string()))
    }
}

pub(super) fn state_text(state: &effect::State) -> &'static str {
    match state {
        effect::State::Prepared => "prepared",
        effect::State::Dispatching => "dispatching",
        effect::State::Unknown => "unknown",
        effect::State::Settled(effect::Outcome::Applied) => "settled_applied",
        effect::State::Settled(effect::Outcome::NotApplied) => "settled_not_applied",
    }
}

pub(super) fn decode_state(value: &str) -> Result<effect::State, String> {
    match value {
        "prepared" => Ok(effect::State::Prepared),
        "dispatching" => Ok(effect::State::Dispatching),
        "unknown" => Ok(effect::State::Unknown),
        "settled_applied" => Ok(effect::State::Settled(effect::Outcome::Applied)),
        "settled_not_applied" => Ok(effect::State::Settled(effect::Outcome::NotApplied)),
        value => Err(format!("unknown effect state {value}")),
    }
}
