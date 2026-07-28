use super::{AttentionDraft, Store, read, write};
use crate::store::inbox::{NoticeDraft, Offer, offer_in};

impl Store {
    pub async fn attend_job(
        &self,
        attention: AttentionDraft<'_>,
        notice: NoticeDraft<'_>,
        gate: usize,
    ) -> Result<Offer, String> {
        let next = attention
            .base
            .checked_add(1)
            .ok_or_else(|| "job attention revision is out of range".to_string())?;
        let revision = i64::try_from(next).map_err(|_| "job attention revision is out of range")?;
        if notice.revision != revision {
            return Err("job attention and inbox revisions do not agree".to_string());
        }
        self.core
            .batch(async |tx| {
                let job = write::relation(tx, "Job", attention.job).await?;
                let current = job.int("attention_revision").ok_or_else(|| {
                    keel::adapt::Error::Adapt("job attention revision missing".into())
                })?;
                let base = i64::try_from(attention.base).map_err(|_| {
                    keel::adapt::Error::Adapt("job attention revision is out of range".into())
                })?;
                if current < base {
                    return Err(keel::adapt::Error::Adapt(
                        "job attention revision has a gap".into(),
                    ));
                }
                if current == base {
                    update(tx, &job, attention, revision).await?;
                }
                offer_in(tx, notice, gate).await
            })
            .await
            .map_err(read::error)
    }
}

async fn update(
    tx: &mut keel::Tx<'_, keel::adapt::db::Sqlite>,
    job: &keel::Row,
    attention: AttentionDraft<'_>,
    revision: i64,
) -> Result<(), keel::adapt::Error> {
    let tick = i64::try_from(attention.tick)
        .map_err(|_| keel::adapt::Error::Adapt("job reminder tick is out of range".into()))?;
    let revision = revision.to_string();
    let tick = tick.to_string();
    let mut fields = vec![
        ("attention_revision", revision.as_str()),
        ("updated", attention.at),
    ];
    if attention.runtime && job.text("runtime_warned").is_none() {
        fields.push(("runtime_warned", attention.at));
    }
    if attention.output && job.text("output_warned").is_none() {
        fields.push(("output_warned", attention.at));
    }
    if attention.reminded {
        fields.push(("last_reminded", attention.at));
        fields.push(("reminder_tick", tick.as_str()));
        if let Some(next) = attention.next {
            fields.push(("next_reminder", next));
        }
    }
    tx.set("Job", job.key(), &fields).await?;
    if attention.reminded && attention.next.is_none() && job.text("next_reminder").is_some() {
        tx.unset("Job", job.key(), &["next_reminder"]).await?;
    }
    Ok(())
}
