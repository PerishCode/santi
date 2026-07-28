use super::{EffectDraft, RedemptionDraft};
use crate::store::tool::{ReplyDraft, put_reply};
use keel::adapt::db::Sqlite;
use keel::{Op, Tx, form};
use santi_model::effect;

pub(super) async fn prepare(
    tx: &mut Tx<'_, Sqlite>,
    draft: EffectDraft<'_>,
    metadata: Option<&str>,
) -> Result<(), keel::adapt::Error> {
    let turn = relation(tx, "Turn", draft.turn).await?;
    let call = call(tx, draft.call, turn.key()).await?;
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
    tx.put("StrandEffect", &fields).await?;
    Ok(())
}

pub(super) async fn shift(
    tx: &mut Tx<'_, Sqlite>,
    tag: &str,
    expected: &[&str],
    state: effect::State,
    error: Option<&str>,
    occurred: &str,
) -> Result<i64, keel::adapt::Error> {
    let row = tx
        .one(&form("StrandEffect").when("tag", Op::Eq, tag))
        .await?
        .ok_or_else(|| keel::adapt::Error::Missing(tag.into()))?;
    let current = row
        .text("state")
        .ok_or_else(|| keel::adapt::Error::Adapt("effect state missing".into()))?;
    if !expected.contains(&current) {
        return Err(keel::adapt::Error::Adapt(format!(
            "effect state {current} cannot transition to {}",
            state_text(&state)
        )));
    }
    let mut fields = vec![("state", state_text(&state)), ("updated", occurred)];
    match state {
        effect::State::Dispatching => fields.push(("dispatched", occurred)),
        effect::State::Settled(_) => fields.push(("settled", occurred)),
        effect::State::Prepared | effect::State::Unknown => {}
    }
    if let Some(error) = error {
        fields.push(("error", error));
    }
    tx.set("StrandEffect", row.key(), &fields).await?;
    Ok(row.key())
}

pub(super) async fn redeem(
    tx: &mut Tx<'_, Sqlite>,
    tag: &str,
    draft: RedemptionDraft<'_>,
    output: Option<&str>,
) -> Result<(), keel::adapt::Error> {
    let row = tx
        .one(&form("StrandEffect").when("tag", Op::Eq, tag))
        .await?
        .ok_or_else(|| keel::adapt::Error::Missing(tag.into()))?;
    let call = relation(tx, "ToolCall", draft.call).await?;
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
        tx,
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
    let key = shift(tx, tag, expected, state, draft.error, draft.occurred).await?;
    tx.set("StrandEffect", key, &[("result", draft.result)])
        .await?;
    Ok(())
}

pub(super) async fn relation(
    tx: &mut Tx<'_, Sqlite>,
    unit: &str,
    tag: &str,
) -> Result<keel::Row, keel::adapt::Error> {
    tx.one(&form(unit).when("tag", Op::Eq, tag))
        .await?
        .ok_or_else(|| keel::adapt::Error::Missing(format!("{unit} {tag}")))
}

async fn call(
    tx: &mut Tx<'_, Sqlite>,
    tag: Option<&str>,
    turn: i64,
) -> Result<Option<String>, keel::adapt::Error> {
    let Some(tag) = tag else {
        return Ok(None);
    };
    let call = relation(tx, "ToolCall", tag).await?;
    if call.int("turn") != Some(turn) {
        return Err(keel::adapt::Error::Adapt(
            "effect call belongs to another turn".into(),
        ));
    }
    Ok(Some(call.key().to_string()))
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
