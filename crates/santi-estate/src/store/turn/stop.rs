use super::{Store, read};
use keel::{Op, form};
use santi_model::turn;

impl Store {
    pub async fn request_stop(
        &self,
        tag: &str,
        cause: turn::Cause,
        requested: &str,
    ) -> Result<Option<turn::Stop>, String> {
        let found = self
            .core
            .batch(async |tx| {
                let Some(turn) = tx.one(&form("Turn").when("tag", Op::Eq, tag)).await? else {
                    return Ok(false);
                };
                let key = turn.key().to_string();
                let terminal = tx
                    .one(&form("TurnCompletion").when("turn", Op::Eq, &key))
                    .await?
                    .is_some()
                    || tx
                        .one(&form("TurnFailure").when("turn", Op::Eq, &key))
                        .await?
                        .is_some();
                if !terminal
                    && tx
                        .one(&form("TurnStop").when("turn", Op::Eq, &key))
                        .await?
                        .is_none()
                {
                    tx.put(
                        "TurnStop",
                        &[
                            ("cause", cause.encode()),
                            ("requested", requested),
                            ("turn", &key),
                        ],
                    )
                    .await?;
                }
                Ok(true)
            })
            .await
            .map_err(read::error)?;
        if !found {
            return Ok(None);
        }
        self.stop(tag).await
    }

    pub async fn settle_stop(&self, tag: &str, settled: &str) -> Result<turn::Stop, String> {
        self.core
            .batch(async |tx| {
                let turn = read::need(tx, "Turn", "tag", tag).await?;
                let stop = tx
                    .one(&form("TurnStop").when("turn", Op::Eq, &turn.to_string()))
                    .await?
                    .ok_or_else(|| keel::adapt::Error::Missing("turn stop".into()))?;
                let key = turn.to_string();
                let terminal = tx
                    .one(&form("TurnCompletion").when("turn", Op::Eq, &key))
                    .await?
                    .is_some()
                    || tx
                        .one(&form("TurnFailure").when("turn", Op::Eq, &key))
                        .await?
                        .is_some();
                if !terminal {
                    return Err(keel::adapt::Error::Adapt(
                        "running turn stop cannot settle".into(),
                    ));
                }
                if stop.text("settled").is_none() {
                    tx.set("TurnStop", stop.key(), &[("settled", settled)])
                        .await?;
                }
                Ok(())
            })
            .await
            .map_err(read::error)?;
        self.stop(tag)
            .await?
            .ok_or_else(|| "settled turn missing".to_string())
    }

    pub async fn stop(&self, tag: &str) -> Result<Option<turn::Stop>, String> {
        let Some(turn) = self.turn(tag).await? else {
            return Ok(None);
        };
        let row = read::one(&self.core, "TurnStop", "turn", &turn_key(self, tag).await?).await?;
        let (cause, requested, settled) = match row {
            Some(row) => (
                Some(decode(
                    row.text("cause")
                        .ok_or_else(|| "turn stop cause missing".to_string())?,
                )?),
                Some(
                    row.text("requested")
                        .ok_or_else(|| "turn stop requested missing".to_string())?
                        .to_string(),
                ),
                row.text("settled").map(str::to_string),
            ),
            None => (None, None, None),
        };
        Ok(Some(turn::Stop {
            accepted: cause.is_some(),
            turn,
            cause,
            requested,
            settled,
        }))
    }
}

async fn turn_key(store: &Store, tag: &str) -> Result<String, String> {
    read::one(&store.core, "Turn", "tag", tag)
        .await?
        .map(|row| row.key().to_string())
        .ok_or_else(|| "turn not found".to_string())
}

pub(super) fn decode(value: &str) -> Result<turn::Cause, String> {
    match value {
        "operator" => Ok(turn::Cause::Operator),
        "shutdown" => Ok(turn::Cause::Shutdown),
        value => Err(format!("unknown turn stop cause {value}")),
    }
}
