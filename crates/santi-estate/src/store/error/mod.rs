use super::{Store, read};
use keel::{Op, Rank, form};
use santi_error::{Fault, Incident, Transition};

mod codec;
mod persist;

pub(in crate::store) struct Resolution<'a> {
    pub key: &'a str,
    pub by: &'a str,
    pub context: serde_json::Value,
    pub now: &'a str,
}

impl Store {
    pub async fn raise(&self, draft: santi_error::Draft, now: &str) -> Result<Fault, String> {
        self.core
            .batch(async |tx| raise_in(tx, draft, now).await)
            .await
            .map_err(read::error)
    }

    pub async fn resolve(
        &self,
        key: &str,
        by: &str,
        context: serde_json::Value,
        now: &str,
    ) -> Result<bool, String> {
        self.core
            .batch(async |tx| {
                resolve_in(
                    tx,
                    Resolution {
                        key,
                        by,
                        context,
                        now,
                    },
                )
                .await
            })
            .await
            .map_err(read::error)
    }

    pub async fn incident(&self, key: &str) -> Result<Option<Incident>, String> {
        read::one(&self.core, "ErrorIncident", "incident_key", key)
            .await?
            .map(|row| codec::incident(&row, false))
            .transpose()
    }

    pub async fn incidents(
        &self,
        scope: &santi_error::Scope,
        limit: usize,
    ) -> Result<Vec<Incident>, String> {
        let rows = self
            .core
            .batch(async |tx| {
                let query = |unit| {
                    form(unit)
                        .when("scope_kind", Op::Eq, &scope.kind)
                        .when("scope_id", Op::Eq, &scope.id)
                        .order("first_seen", Rank::Desc)
                        .order("tag", Rank::Desc)
                };
                let active = tx.ask(&query("ErrorIncident")).await?;
                let resolved = tx.ask(&query("ResolvedIncident")).await?;
                Ok((active.rows().to_vec(), resolved.rows().to_vec()))
            })
            .await
            .map_err(read::error)?;
        let mut incidents = rows
            .0
            .iter()
            .map(|row| codec::incident(row, false))
            .chain(rows.1.iter().map(|row| codec::incident(row, true)))
            .collect::<Result<Vec<_>, _>>()?;
        incidents.sort_by(|left, right| {
            right
                .first
                .seen
                .cmp(&left.first.seen)
                .then_with(|| right.id.cmp(&left.id))
        });
        incidents.truncate(limit.clamp(1, 1000));
        Ok(incidents)
    }

    pub async fn active_incident_count(&self, code: &str) -> Result<usize, String> {
        let rows = self
            .core
            .ask(&form("ErrorIncident").when("code", Op::Eq, code))
            .await
            .map_err(read::error)?;
        Ok(rows.rows().len())
    }

    pub async fn pending_errors(&self, limit: usize) -> Result<Vec<Transition>, String> {
        let rows = self
            .core
            .batch(async |tx| {
                let transitions = tx
                    .ask(
                        &form("ErrorTransition")
                            .order("created", Rank::Asc)
                            .order("tag", Rank::Asc),
                    )
                    .await?;
                let acknowledgements = tx.live("ErrorAcknowledgement").await?;
                Ok((transitions.rows().to_vec(), acknowledgements))
            })
            .await
            .map_err(read::error)?;
        rows.0
            .iter()
            .filter(|transition| {
                !rows
                    .1
                    .iter()
                    .any(|ack| ack.int("transition") == Some(transition.key()))
            })
            .take(limit)
            .map(codec::transition)
            .collect()
    }

    pub async fn deliver_error(&self, transition: &str, delivered: &str) -> Result<(), String> {
        self.core
            .batch(async |tx| {
                let Some(row) = tx
                    .one(&form("ErrorTransition").when("tag", Op::Eq, transition))
                    .await?
                else {
                    return Ok(());
                };
                if tx
                    .one(&form("ErrorAcknowledgement").when(
                        "transition",
                        Op::Eq,
                        &row.key().to_string(),
                    ))
                    .await?
                    .is_none()
                {
                    tx.put(
                        "ErrorAcknowledgement",
                        &[
                            ("delivered", delivered),
                            ("transition", &row.key().to_string()),
                        ],
                    )
                    .await?;
                }
                Ok(())
            })
            .await
            .map_err(read::error)
    }
}

pub(in crate::store) async fn raise_in(
    tx: &mut keel::Tx<'_, keel::adapt::db::Sqlite>,
    draft: santi_error::Draft,
    now: &str,
) -> Result<Fault, keel::adapt::Error> {
    if draft.key != draft.descriptor.key(&draft.scope.kind, &draft.scope.id) {
        return Err(keel::adapt::Error::Adapt(
            "incident key does not match its descriptor and scope".into(),
        ));
    }
    let row = tx
        .one(&form("ErrorIncident").when("incident_key", Op::Eq, &draft.key))
        .await?;
    let existing = row
        .as_ref()
        .map(|row| codec::incident(row, false).map_err(adapt))
        .transpose()?;
    let mutation = santi_error::engine().open(existing.as_ref(), draft, now);
    persist::opened(tx, &mutation, existing.is_some()).await?;
    Ok(mutation.error)
}

pub(in crate::store) async fn resolve_in(
    tx: &mut keel::Tx<'_, keel::adapt::db::Sqlite>,
    resolution: Resolution<'_>,
) -> Result<bool, keel::adapt::Error> {
    let Some(row) = tx
        .one(&form("ErrorIncident").when("incident_key", Op::Eq, resolution.key))
        .await?
    else {
        return Ok(false);
    };
    let active = codec::incident(&row, false).map_err(adapt)?;
    let mutation =
        santi_error::engine().resolve(&active, resolution.by, resolution.context, resolution.now);
    persist::resolved(tx, row.key(), &mutation).await?;
    Ok(true)
}

fn adapt(error: String) -> keel::adapt::Error {
    keel::adapt::Error::Adapt(error)
}
