use super::{Store, fail, read};
use crate::store::error;
use keel::{Op, form};
use santi_error::Fault;
use santi_model::turn;

pub struct ClassifiedFailureDraft<'a> {
    pub turn: &'a str,
    pub detail: &'a str,
    pub incident: santi_error::Draft,
    pub occurred: &'a str,
}

pub struct ClassifiedFailure {
    pub turn: turn::Turn,
    pub fault: Fault,
}

impl Store {
    pub async fn fail_classified(
        &self,
        draft: ClassifiedFailureDraft<'_>,
    ) -> Result<ClassifiedFailure, String> {
        let tag = draft.turn.to_string();
        let fault = self
            .core
            .batch(async |tx| {
                let turn = tx
                    .one(&form("Turn").when("tag", Op::Eq, draft.turn))
                    .await?
                    .ok_or_else(|| keel::adapt::Error::Missing(draft.turn.into()))?;
                let strand = turn.int("strand").ok_or_else(|| {
                    keel::adapt::Error::Adapt("classified failure strand missing".into())
                })?;
                let strand = tx
                    .one(&form("Strand").when("id", Op::Eq, &strand.to_string()))
                    .await?
                    .ok_or_else(|| keel::adapt::Error::Missing("failure strand".into()))?;
                if draft.incident.scope.kind != "strand"
                    || strand.text("tag") != Some(draft.incident.scope.id.as_str())
                {
                    return Err(keel::adapt::Error::Adapt(
                        "classified failure incident belongs to another strand".into(),
                    ));
                }
                let fault = error::raise_in(tx, draft.incident, draft.occurred).await?;
                fail(
                    tx,
                    draft.turn,
                    draft.detail,
                    fault.incident.as_deref(),
                    draft.occurred,
                )
                .await?;
                Ok(fault)
            })
            .await
            .map_err(read::error)?;
        let turn = self
            .turn(&tag)
            .await?
            .ok_or_else(|| "classified failure turn missing".to_string())?;
        Ok(ClassifiedFailure { turn, fault })
    }
}
