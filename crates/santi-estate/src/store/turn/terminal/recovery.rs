use super::{InterruptionDraft, fail, terminal};
use crate::store::turn::stop;
use crate::store::{error, write};
use keel::adapt::db::Sqlite;
use keel::{Op, Rank, Row, Tx, form};
use santi_error::Ruled;
use santi_model::{message, turn};

pub(super) async fn interrupt(
    tx: &mut Tx<'_, Sqlite>,
    draft: &InterruptionDraft<'_>,
) -> Result<Option<String>, keel::adapt::Error> {
    let turn = tx
        .one(&form("Turn").when("tag", Op::Eq, draft.turn))
        .await?
        .ok_or_else(|| keel::adapt::Error::Missing(draft.turn.into()))?;
    let existing = stop(tx, &turn, None, draft.occurred).await?;
    if terminal(tx, turn.key()).await? {
        settle(tx, existing.as_ref(), draft.occurred).await?;
        return Ok(None);
    }
    let stop = match existing {
        Some(stop) => Some(stop),
        None => stop(tx, &turn, Some(draft.cause), draft.occurred).await?,
    };
    let stop = stop.ok_or_else(|| keel::adapt::Error::Adapt("turn stop missing".into()))?;
    let cause = cause(&stop)?;
    let notice = terminate(tx, &turn, Some(cause), draft.actor, draft.occurred).await?;
    settle(tx, Some(&stop), draft.occurred).await?;
    Ok(notice)
}

pub(super) async fn recover(
    tx: &mut Tx<'_, Sqlite>,
    actor: &str,
    occurred: &str,
) -> Result<usize, keel::adapt::Error> {
    let turns = tx
        .ask(
            &form("Turn")
                .order("created", Rank::Asc)
                .order("tag", Rank::Asc),
        )
        .await?
        .rows()
        .to_vec();
    let mut recovered = 0;
    for turn in turns {
        if terminal(tx, turn.key()).await? {
            continue;
        }
        let stop = stop(tx, &turn, None, occurred).await?;
        let cause = stop.as_ref().map(cause).transpose()?;
        terminate(tx, &turn, cause, actor, occurred).await?;
        settle(tx, stop.as_ref(), occurred).await?;
        recovered += 1;
    }
    Ok(recovered)
}

async fn terminate(
    tx: &mut Tx<'_, Sqlite>,
    turn: &Row,
    cause: Option<turn::Cause>,
    actor: &str,
    occurred: &str,
) -> Result<Option<String>, keel::adapt::Error> {
    let tag = text(turn, "tag")?;
    let strand = related(tx, turn, "strand", "turn strand").await?;
    let strand_tag = text(&strand, "tag")?;
    let (detail, incident) = match cause {
        Some(cause) => (format!("interrupted by {}", cause.encode()), None),
        None => {
            let detail = "interrupted by restart".to_string();
            let descriptor = turn::Error::Runtime.descriptor();
            let fault = error::raise_in(
                tx,
                santi_error::Draft {
                    key: descriptor.key("strand", strand_tag),
                    descriptor,
                    scope: santi_error::Scope::new("strand", strand_tag),
                    source: santi_error::Source::new("santi-core", "turn.restart_reconcile"),
                    message: "turn failed inside the runtime".to_string(),
                    context: serde_json::json!({
                        "schema": "santi.error.runtime_turn.v1",
                        "turn": tag,
                        "operation": "turn.restart_reconcile",
                        "detail": detail,
                        "trace": format!("log://turn/{tag}"),
                    }),
                },
                occurred,
            )
            .await?;
            (detail, fault.incident)
        }
    };
    fail(tx, tag, &detail, incident.as_deref(), occurred).await?;
    match cause {
        Some(cause) => notice(tx, &strand, tag, cause, actor, occurred)
            .await
            .map(Some),
        None => Ok(None),
    }
}

async fn stop(
    tx: &mut Tx<'_, Sqlite>,
    turn: &Row,
    requested: Option<turn::Cause>,
    occurred: &str,
) -> Result<Option<Row>, keel::adapt::Error> {
    let key = turn.key().to_string();
    if let Some(stop) = tx.one(&form("TurnStop").when("turn", Op::Eq, &key)).await? {
        return Ok(Some(stop));
    }
    let Some(requested) = requested else {
        return Ok(None);
    };
    let key = tx
        .put(
            "TurnStop",
            &[
                ("cause", requested.encode()),
                ("requested", occurred),
                ("turn", &key),
            ],
        )
        .await?;
    tx.one(&form("TurnStop").when("id", Op::Eq, &key.to_string()))
        .await
}

async fn settle(
    tx: &mut Tx<'_, Sqlite>,
    stop: Option<&Row>,
    occurred: &str,
) -> Result<(), keel::adapt::Error> {
    if let Some(stop) = stop
        && stop.text("settled").is_none()
    {
        tx.set("TurnStop", stop.key(), &[("settled", occurred)])
            .await?;
    }
    Ok(())
}

async fn notice(
    tx: &mut Tx<'_, Sqlite>,
    strand: &Row,
    turn: &str,
    cause: turn::Cause,
    actor: &str,
    occurred: &str,
) -> Result<String, keel::adapt::Error> {
    let tag = santi_model::tag("msg");
    let content = serde_json::to_string(&message::Content::text(format!(
        "<system_message>\nThe previous turn ({turn}) was interrupted by {}. Its partial output and external effects may be incomplete; inspect durable effect state before retrying.\n</system_message>",
        cause.encode()
    )))
    .map_err(|error| keel::adapt::Error::Adapt(error.to_string()))?;
    tx.put(
        "Message",
        &[
            ("tag", tag.as_str()),
            ("actor_type", "system"),
            ("actor", actor),
            ("kind", "santi_system"),
            ("content", content.as_str()),
            ("state", "fixed"),
            ("request", "false"),
            ("created", occurred),
            ("updated", occurred),
        ],
    )
    .await?;
    let strand = tx
        .one(&form("Strand").when("id", Op::Eq, &strand.key().to_string()))
        .await?
        .ok_or_else(|| keel::adapt::Error::Missing("interruption strand".into()))?;
    write::append(tx, &strand, "message", &tag, occurred).await?;
    Ok(tag)
}

async fn related(
    tx: &mut Tx<'_, Sqlite>,
    row: &Row,
    field: &str,
    missing: &str,
) -> Result<Row, keel::adapt::Error> {
    let key = row
        .int(field)
        .ok_or_else(|| keel::adapt::Error::Adapt(format!("{missing} missing")))?;
    tx.one(&form("Strand").when("id", Op::Eq, &key.to_string()))
        .await?
        .ok_or_else(|| keel::adapt::Error::Missing(missing.into()))
}

fn cause(stop: &Row) -> Result<turn::Cause, keel::adapt::Error> {
    stop.text("cause")
        .ok_or_else(|| keel::adapt::Error::Adapt("turn stop cause missing".into()))
        .and_then(|cause| stop::decode(cause).map_err(keel::adapt::Error::Adapt))
}

fn text<'a>(row: &'a Row, field: &str) -> Result<&'a str, keel::adapt::Error> {
    row.text(field)
        .ok_or_else(|| keel::adapt::Error::Adapt(format!("turn {field} missing")))
}
