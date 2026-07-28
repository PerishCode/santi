use super::{Store, read};
use crate::store::effect;
use keel::adapt::db::Sqlite;
use keel::{Op, Row, Tx, form};
use santi_model::{message, receipt, turn};

mod failure;
mod recovery;
mod success;

pub use failure::{ClassifiedFailure, ClassifiedFailureDraft};
pub use success::{Completion, CompletionDraft};

pub struct InterruptionDraft<'a> {
    pub turn: &'a str,
    pub cause: turn::Cause,
    pub actor: &'a str,
    pub occurred: &'a str,
}

pub struct Interruption {
    pub stop: turn::Stop,
    pub notice: Option<message::Placed>,
}

impl Store {
    pub async fn interrupt_turn(
        &self,
        draft: InterruptionDraft<'_>,
    ) -> Result<Interruption, String> {
        let notice = self
            .core
            .batch(async |tx| recovery::interrupt(tx, &draft).await)
            .await
            .map_err(read::error)?;
        let stop = self
            .stop(draft.turn)
            .await?
            .ok_or_else(|| "interrupted turn missing".to_string())?;
        let notice = match notice {
            Some(tag) => Some(
                self.message(&tag)
                    .await?
                    .ok_or_else(|| "interruption notice missing".to_string())?,
            ),
            None => None,
        };
        Ok(Interruption { stop, notice })
    }

    pub async fn recover_turns(&self, actor: &str, occurred: &str) -> Result<usize, String> {
        self.core
            .batch(async |tx| recovery::recover(tx, actor, occurred).await)
            .await
            .map_err(read::error)
    }
}

pub(in crate::store) async fn complete(
    tx: &mut Tx<'_, Sqlite>,
    tag: &str,
    to: i64,
    finished: &str,
) -> Result<(), keel::adapt::Error> {
    let turn = eligible(tx, tag).await?;
    let key = turn.key().to_string();
    if tx
        .one(&form("TurnStop").when("turn", Op::Eq, &key))
        .await?
        .is_some()
    {
        return Err(keel::adapt::Error::Adapt(
            "stopped turn cannot complete".into(),
        ));
    }
    tx.put(
        "TurnCompletion",
        &[
            ("to", &to.to_string()),
            ("finished", finished),
            ("turn", &key),
        ],
    )
    .await?;
    crate::store::inbox::receipt::close(tx, tag, receipt::State::Completed, None, finished).await?;
    tx.set("Turn", turn.key(), &[("updated", finished)]).await
}

pub(in crate::store) async fn fail(
    tx: &mut Tx<'_, Sqlite>,
    tag: &str,
    detail: &str,
    incident: Option<&str>,
    finished: &str,
) -> Result<(), keel::adapt::Error> {
    let turn = eligible(tx, tag).await?;
    tx.put(
        "TurnFailure",
        &[
            ("error", detail),
            ("finished", finished),
            ("turn", &turn.key().to_string()),
        ],
    )
    .await?;
    effect::reconcile_in(tx, tag, finished).await?;
    crate::store::inbox::receipt::close(tx, tag, receipt::State::Failed, incident, finished)
        .await?;
    tx.set("Turn", turn.key(), &[("updated", finished)]).await
}

async fn eligible(tx: &mut Tx<'_, Sqlite>, tag: &str) -> Result<Row, keel::adapt::Error> {
    let turn = tx
        .one(&form("Turn").when("tag", Op::Eq, tag))
        .await?
        .ok_or_else(|| keel::adapt::Error::Missing(tag.into()))?;
    if terminal(tx, turn.key()).await? {
        return Err(keel::adapt::Error::Adapt("turn already finished".into()));
    }
    Ok(turn)
}

pub(super) async fn terminal(
    tx: &mut Tx<'_, Sqlite>,
    turn: i64,
) -> Result<bool, keel::adapt::Error> {
    let key = turn.to_string();
    Ok(tx
        .one(&form("TurnCompletion").when("turn", Op::Eq, &key))
        .await?
        .is_some()
        || tx
            .one(&form("TurnFailure").when("turn", Op::Eq, &key))
            .await?
            .is_some())
}
