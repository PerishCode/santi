use super::{CapabilityDraft, JobDraft, TransitionDraft};
use keel::adapt::db::Sqlite;
use keel::{Op, Row, Tx, form};
use santi_model::job;

mod support;
use support::{Origin, adapt, key, signed, tag, validate};

pub(super) struct Writer<'a, 'tx>(&'a mut Tx<'tx, Sqlite>);

pub(super) async fn capability(
    tx: &mut Tx<'_, Sqlite>,
    draft: CapabilityDraft<'_>,
) -> Result<(), keel::adapt::Error> {
    let mut writer = Writer(tx);
    if draft.expires < 1 {
        return Err(adapt("job capability expiry must be positive"));
    }
    let soul = writer.relation("Soul", draft.soul).await?;
    let strand = writer.relation("Strand", draft.strand).await?;
    let turn = writer.relation("Turn", draft.turn).await?;
    let call = writer.relation("ToolCall", draft.call).await?;
    let effect = writer.relation("StrandEffect", draft.effect).await?;
    validate(Origin {
        soul: &soul,
        strand: &strand,
        turn: &turn,
        call: &call,
        effect: &effect,
    })?;
    writer
        .0
        .put(
            "JobCapability",
            &[
                ("digest", draft.digest),
                ("expires", &draft.expires.to_string()),
                ("created", draft.created),
                ("soul", &soul.key().to_string()),
                ("strand", &strand.key().to_string()),
                ("turn", &turn.key().to_string()),
                ("call", &call.key().to_string()),
                ("effect", &effect.key().to_string()),
            ],
        )
        .await?;
    Ok(())
}

pub(super) async fn prepare(
    tx: &mut Tx<'_, Sqlite>,
    digest: &str,
    draft: JobDraft<'_>,
    now_millis: i64,
) -> Result<(bool, String), keel::adapt::Error> {
    let mut writer = Writer(tx);
    let capability = writer
        .0
        .one(&form("JobCapability").when("digest", Op::Eq, digest))
        .await?
        .ok_or_else(|| adapt("invalid job capability"))?;
    if let Some(job) = capability.int("consumed") {
        if capability.text("request_sha256") != Some(draft.request_sha256) {
            return Err(adapt("job capability conflicts with its accepted request"));
        }
        let job = writer
            .0
            .one(&form("Job").when("id", Op::Eq, &job.to_string()))
            .await?
            .ok_or_else(|| adapt("consumed job capability has no job record"))?;
        return Ok((false, tag(&job)?));
    }
    let expires = capability
        .int("expires")
        .ok_or_else(|| adapt("job capability expiry missing"))?;
    if now_millis > expires {
        return Err(adapt("job capability expired"));
    }
    let job = writer.put_job(&capability, draft).await?;
    writer
        .0
        .set(
            "JobCapability",
            capability.key(),
            &[
                ("request_sha256", draft.request_sha256),
                ("consumed", &job.to_string()),
            ],
        )
        .await?;
    Ok((true, draft.tag.to_string()))
}

pub(super) async fn accept(
    tx: &mut Tx<'_, Sqlite>,
    tag: &str,
    occurred: &str,
) -> Result<(), keel::adapt::Error> {
    let mut writer = Writer(tx);
    let job = writer.relation("Job", tag).await?;
    if job.text("state") == Some("submitting") {
        writer
            .0
            .set(
                "Job",
                job.key(),
                &[
                    ("state", "accepted"),
                    ("accepted", occurred),
                    ("updated", occurred),
                ],
            )
            .await?;
    }
    Ok(())
}

impl<'a, 'tx> Writer<'a, 'tx> {
    pub fn new(tx: &'a mut Tx<'tx, Sqlite>) -> Self {
        Self(tx)
    }

    pub async fn transition(
        &mut self,
        tag: &str,
        draft: TransitionDraft<'_>,
    ) -> Result<(), keel::adapt::Error> {
        let job = self.relation("Job", tag).await?;
        let mut fields = vec![("state", state(&draft.state)), ("updated", draft.occurred)];
        if let Some(reason) = draft.reason {
            fields.push(("reason", reason));
        }
        let exit = draft.exit_code.map(|exit| exit.to_string());
        if let Some(exit) = exit.as_deref() {
            fields.push(("exit_code", exit));
        }
        let started = (draft.state == job::State::Running && job.text("started").is_none())
            .then_some(draft.occurred);
        if let Some(started) = started {
            fields.push(("started", started));
        }
        let millis = (draft.state == job::State::Running && job.int("started_millis").is_none())
            .then_some(draft.started_millis)
            .flatten()
            .map(|millis| millis.to_string());
        if let Some(millis) = millis.as_deref() {
            fields.push(("started_millis", millis));
        }
        if draft.state == job::State::Running
            && job.text("next_reminder").is_none()
            && let Some(next) = draft.next_reminder
        {
            fields.push(("next_reminder", next));
        }
        if draft.state.terminal() && job.text("finished").is_none() {
            fields.push(("finished", draft.occurred));
        }
        self.0.set("Job", job.key(), &fields).await?;
        self.clear_transition(&job, &draft).await
    }

    pub async fn acknowledge(
        &mut self,
        tag: &str,
        occurred: &str,
    ) -> Result<(), keel::adapt::Error> {
        let job = self.relation("Job", tag).await?;
        if job.text("acknowledged").is_none() {
            self.0
                .set(
                    "Job",
                    job.key(),
                    &[("acknowledged", occurred), ("updated", occurred)],
                )
                .await?;
        }
        Ok(())
    }

    pub async fn relation(&mut self, unit: &str, tag: &str) -> Result<Row, keel::adapt::Error> {
        self.0
            .one(&form(unit).when("tag", Op::Eq, tag))
            .await?
            .ok_or_else(|| keel::adapt::Error::Missing(format!("{unit} {tag}")))
    }
}

pub(super) fn state(state: &job::State) -> &'static str {
    match state {
        job::State::Submitting => "submitting",
        job::State::Accepted => "accepted",
        job::State::Running => "running",
        job::State::Cancelling => "cancelling",
        job::State::Succeeded => "succeeded",
        job::State::Failed => "failed",
        job::State::TimedOut => "timed_out",
        job::State::Cancelled => "cancelled",
        job::State::Unknown => "unknown",
    }
}

impl Writer<'_, '_> {
    async fn put_job(
        &mut self,
        capability: &Row,
        draft: JobDraft<'_>,
    ) -> Result<i64, keel::adapt::Error> {
        let timeout = signed(draft.timeout_seconds, "job timeout")?;
        let output = signed(draft.output_limit_bytes, "job output limit")?;
        let remind = draft
            .remind_every_seconds
            .map(|value| signed(value, "job reminder interval"))
            .transpose()?;
        let strand = key(capability, "strand")?;
        let turn = key(capability, "turn")?;
        let call = key(capability, "call")?;
        let effect = key(capability, "effect")?;
        let mut fields = vec![
            ("tag", draft.tag),
            ("description", draft.description),
            ("command", draft.command),
            ("timeout_seconds", timeout.as_str()),
            ("output_limit_bytes", output.as_str()),
            ("request_sha256", draft.request_sha256),
            (
                "capability_sha256",
                capability
                    .text("digest")
                    .ok_or_else(|| adapt("capability digest missing"))?,
            ),
            ("generation", draft.generation),
            ("supervisor_ref", draft.supervisor_ref),
            ("created", draft.created),
            ("updated", draft.created),
            ("strand", strand.as_str()),
            ("turn", turn.as_str()),
            ("call", call.as_str()),
            ("effect", effect.as_str()),
        ];
        if let Some(cwd) = draft.cwd {
            fields.push(("cwd", cwd));
        }
        if let Some(remind) = remind.as_deref() {
            fields.push(("remind_every_seconds", remind));
        }
        self.0.put("Job", &fields).await
    }

    async fn clear_transition(
        &mut self,
        job: &Row,
        draft: &TransitionDraft<'_>,
    ) -> Result<(), keel::adapt::Error> {
        let mut unset = Vec::new();
        if draft.reason.is_none() && job.text("reason").is_some() {
            unset.push("reason");
        }
        if draft.exit_code.is_none() && job.int("exit_code").is_some() {
            unset.push("exit_code");
        }
        if draft.state.terminal() && job.text("next_reminder").is_some() {
            unset.push("next_reminder");
        }
        if !unset.is_empty() {
            self.0.unset("Job", job.key(), &unset).await?;
        }
        Ok(())
    }
}
