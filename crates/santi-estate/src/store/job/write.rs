use super::{CapabilityDraft, JobDraft, TransitionDraft};
use keel::adapt::db::Sqlite;
use keel::{Op, Row, Tx, form};
use santi_model::job;

pub(super) async fn capability(
    tx: &mut Tx<'_, Sqlite>,
    draft: CapabilityDraft<'_>,
) -> Result<(), keel::adapt::Error> {
    if draft.expires < 1 {
        return Err(adapt("job capability expiry must be positive"));
    }
    let soul = relation(tx, "Soul", draft.soul).await?;
    let strand = relation(tx, "Strand", draft.strand).await?;
    let turn = relation(tx, "Turn", draft.turn).await?;
    let call = relation(tx, "ToolCall", draft.call).await?;
    let effect = relation(tx, "StrandEffect", draft.effect).await?;
    validate_origin(&soul, &strand, &turn, &call, &effect)?;
    tx.put(
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
    let capability = tx
        .one(&form("JobCapability").when("digest", Op::Eq, digest))
        .await?
        .ok_or_else(|| adapt("invalid job capability"))?;
    if let Some(job) = capability.int("consumed") {
        if capability.text("request_sha256") != Some(draft.request_sha256) {
            return Err(adapt("job capability conflicts with its accepted request"));
        }
        let job = tx
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
    let job = put_job(tx, &capability, draft).await?;
    tx.set(
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
    let job = relation(tx, "Job", tag).await?;
    if job.text("state") == Some("submitting") {
        tx.set(
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

pub(super) async fn transition(
    tx: &mut Tx<'_, Sqlite>,
    tag: &str,
    draft: TransitionDraft<'_>,
) -> Result<(), keel::adapt::Error> {
    let job = relation(tx, "Job", tag).await?;
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
    tx.set("Job", job.key(), &fields).await?;
    clear_transition(tx, &job, &draft).await
}

pub(super) async fn acknowledge(
    tx: &mut Tx<'_, Sqlite>,
    tag: &str,
    occurred: &str,
) -> Result<(), keel::adapt::Error> {
    let job = relation(tx, "Job", tag).await?;
    if job.text("acknowledged").is_none() {
        tx.set(
            "Job",
            job.key(),
            &[("acknowledged", occurred), ("updated", occurred)],
        )
        .await?;
    }
    Ok(())
}

pub(super) async fn relation(
    tx: &mut Tx<'_, Sqlite>,
    unit: &str,
    tag: &str,
) -> Result<Row, keel::adapt::Error> {
    tx.one(&form(unit).when("tag", Op::Eq, tag))
        .await?
        .ok_or_else(|| keel::adapt::Error::Missing(format!("{unit} {tag}")))
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

fn validate_origin(
    soul: &Row,
    strand: &Row,
    turn: &Row,
    call: &Row,
    effect: &Row,
) -> Result<(), keel::adapt::Error> {
    if strand.int("soul") != Some(soul.key())
        || turn.int("strand") != Some(strand.key())
        || call.int("turn") != Some(turn.key())
        || effect.int("turn") != Some(turn.key())
        || effect.int("call") != Some(call.key())
    {
        return Err(adapt("job capability origin is inconsistent"));
    }
    Ok(())
}

async fn put_job(
    tx: &mut Tx<'_, Sqlite>,
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
    tx.put("Job", &fields).await
}

async fn clear_transition(
    tx: &mut Tx<'_, Sqlite>,
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
        tx.unset("Job", job.key(), &unset).await?;
    }
    Ok(())
}

fn signed(value: u64, label: &str) -> Result<String, keel::adapt::Error> {
    i64::try_from(value)
        .map(|value| value.to_string())
        .map_err(|_| adapt(&format!("{label} is out of range")))
}

fn key(row: &Row, relation: &str) -> Result<String, keel::adapt::Error> {
    row.int(relation)
        .map(|key| key.to_string())
        .ok_or_else(|| adapt("job capability relation missing"))
}

fn tag(row: &Row) -> Result<String, keel::adapt::Error> {
    row.text("tag")
        .map(str::to_string)
        .ok_or_else(|| adapt("job tag missing"))
}

fn adapt(message: &str) -> keel::adapt::Error {
    keel::adapt::Error::Adapt(message.to_string())
}
