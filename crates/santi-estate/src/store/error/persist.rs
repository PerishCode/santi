use keel::Tx;
use keel::adapt::db::Sqlite;
use santi_error::{Incident, Mutation, Transition};

struct Writer<'a, 'tx>(&'a mut Tx<'tx, Sqlite>);

pub(super) async fn opened(
    tx: &mut Tx<'_, Sqlite>,
    mutation: &Mutation,
    existing: bool,
) -> Result<(), keel::adapt::Error> {
    let mut writer = Writer(tx);
    if existing {
        writer.update_active(&mutation.incident).await?;
    } else {
        writer
            .put_incident("ErrorIncident", &mutation.incident, None)
            .await?;
    }
    if let Some(transition) = mutation.transition.as_ref() {
        writer.put_transition(transition).await?;
    }
    Ok(())
}

pub(super) async fn resolved(
    tx: &mut Tx<'_, Sqlite>,
    active: i64,
    mutation: &Mutation,
) -> Result<(), keel::adapt::Error> {
    let mut writer = Writer(tx);
    writer.put_resolved(&mutation.incident).await?;
    let transition = mutation
        .transition
        .as_ref()
        .ok_or_else(|| keel::adapt::Error::Adapt("resolution transition missing".into()))?;
    writer.put_transition(transition).await?;
    writer.0.end("ErrorIncident", active).await
}

impl Writer<'_, '_> {
    async fn update_active(&mut self, incident: &Incident) -> Result<(), keel::adapt::Error> {
        let row = self
            .0
            .one(&keel::form("ErrorIncident").when("incident_key", keel::Op::Eq, &incident.key))
            .await?
            .ok_or_else(|| keel::adapt::Error::Missing(incident.key.clone()))?;
        let context = json(&incident.latest.context)?;
        self.0
            .set(
                "ErrorIncident",
                row.key(),
                &[
                    ("latest_source_component", &incident.latest.source.component),
                    ("latest_source_operation", &incident.latest.source.operation),
                    ("latest_message", &incident.latest.message),
                    ("latest_context", &context),
                    ("occurrence_count", &incident.occurrences.to_string()),
                    ("last_seen", &incident.latest.seen),
                ],
            )
            .await
    }

    async fn put_incident(
        &mut self,
        unit: &str,
        incident: &Incident,
        resolution: Option<&santi_error::Resolution>,
    ) -> Result<(), keel::adapt::Error> {
        let exposure = json(&incident.exposure)?;
        let first_context = json(&incident.first.context)?;
        let latest_context = json(&incident.latest.context)?;
        let occurrences = incident.occurrences.to_string();
        let revision = incident.revision.to_string();
        let mut fields = vec![
            ("tag", incident.id.as_str()),
            ("incident_key", incident.key.as_str()),
            ("code", incident.code.as_str()),
            ("category", incident.category.db()),
            ("severity", incident.severity.db()),
            ("retry", incident.retry.db()),
            ("exposure", exposure.as_str()),
            ("scope_kind", incident.scope.kind.as_str()),
            ("scope_id", incident.scope.id.as_str()),
            ("source_component", incident.first.source.component.as_str()),
            ("source_operation", incident.first.source.operation.as_str()),
            (
                "latest_source_component",
                incident.latest.source.component.as_str(),
            ),
            (
                "latest_source_operation",
                incident.latest.source.operation.as_str(),
            ),
            ("message", incident.first.message.as_str()),
            ("latest_message", incident.latest.message.as_str()),
            ("context", first_context.as_str()),
            ("latest_context", latest_context.as_str()),
            ("occurrence_count", occurrences.as_str()),
            ("revision", revision.as_str()),
            ("first_seen", incident.first.seen.as_str()),
            ("last_seen", incident.latest.seen.as_str()),
        ];
        if let Some(resolution) = resolution {
            fields.push(("resolved", resolution.at.as_str()));
            if let Some(by) = resolution.by.as_deref() {
                fields.push(("resolved_by", by));
            }
        }
        self.0.put(unit, &fields).await?;
        Ok(())
    }

    async fn put_resolved(&mut self, incident: &Incident) -> Result<(), keel::adapt::Error> {
        let resolution = incident
            .resolution
            .as_ref()
            .ok_or_else(|| keel::adapt::Error::Adapt("resolution missing".into()))?;
        self.put_incident("ResolvedIncident", incident, Some(resolution))
            .await
    }

    async fn put_transition(&mut self, transition: &Transition) -> Result<(), keel::adapt::Error> {
        let payload = json(transition)?;
        self.0
            .put(
                "ErrorTransition",
                &[
                    ("tag", &transition.id),
                    ("incident", &transition.incident),
                    ("revision", &transition.revision.to_string()),
                    ("kind", transition.kind.db()),
                    ("payload", &payload),
                    ("created", &transition.occurred),
                ],
            )
            .await?;
        Ok(())
    }
}

fn json(value: &impl serde::Serialize) -> Result<String, keel::adapt::Error> {
    serde_json::to_string(value).map_err(|error| keel::adapt::Error::Adapt(error.to_string()))
}
