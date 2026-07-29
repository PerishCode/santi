use super::{Failure, InterruptionDraft, fail, terminal};
use crate::store::turn::stop;
use crate::store::{error, write};
use keel::adapt::db::Sqlite;
use keel::{Op, Rank, Row, Tx, form};
use santi_error::Ruled;
use santi_model::{message, turn};

struct Termination<'a> {
    turn: &'a Row,
    cause: Option<turn::Cause>,
    actor: &'a str,
    occurred: &'a str,
}

struct Notice<'a> {
    strand: &'a Row,
    turn: &'a str,
    cause: turn::Cause,
    actor: &'a str,
    occurred: &'a str,
}

struct Writer<'a, 'tx>(&'a mut Tx<'tx, Sqlite>);

pub(super) async fn interrupt(
    tx: &mut Tx<'_, Sqlite>,
    draft: &InterruptionDraft<'_>,
) -> Result<Option<String>, keel::adapt::Error> {
    let turn = tx
        .one(&form("Turn").when("tag", Op::Eq, draft.turn))
        .await?
        .ok_or_else(|| keel::adapt::Error::Missing(draft.turn.into()))?;
    let existing = Writer(tx).stop(&turn, None, draft.occurred).await?;
    if terminal(tx, turn.key()).await? {
        Writer(tx).settle(existing.as_ref(), draft.occurred).await?;
        return Ok(None);
    }
    let stop = match existing {
        Some(stop) => Some(stop),
        None => {
            Writer(tx)
                .stop(&turn, Some(draft.cause), draft.occurred)
                .await?
        }
    };
    let stop = stop.ok_or_else(|| keel::adapt::Error::Adapt("turn stop missing".into()))?;
    let cause = cause(&stop)?;
    let notice = terminate(
        tx,
        Termination {
            turn: &turn,
            cause: Some(cause),
            actor: draft.actor,
            occurred: draft.occurred,
        },
    )
    .await?;
    Writer(tx).settle(Some(&stop), draft.occurred).await?;
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
        let stop = Writer(tx).stop(&turn, None, occurred).await?;
        let cause = stop.as_ref().map(cause).transpose()?;
        terminate(
            tx,
            Termination {
                turn: &turn,
                cause,
                actor,
                occurred,
            },
        )
        .await?;
        Writer(tx).settle(stop.as_ref(), occurred).await?;
        recovered += 1;
    }
    Ok(recovered)
}

async fn terminate(
    tx: &mut Tx<'_, Sqlite>,
    termination: Termination<'_>,
) -> Result<Option<String>, keel::adapt::Error> {
    let tag = text(termination.turn, "tag")?;
    let strand = Writer(tx)
        .related(termination.turn, "strand", "turn strand")
        .await?;
    let strand_tag = text(&strand, "tag")?;
    let (detail, incident) = match termination.cause {
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
                termination.occurred,
            )
            .await?;
            (detail, fault.incident)
        }
    };
    fail(
        tx,
        Failure {
            turn: tag,
            detail: &detail,
            incident: incident.as_deref(),
            finished: termination.occurred,
        },
    )
    .await?;
    match termination.cause {
        Some(cause) => Writer(tx)
            .notice(Notice {
                strand: &strand,
                turn: tag,
                cause,
                actor: termination.actor,
                occurred: termination.occurred,
            })
            .await
            .map(Some),
        None => Ok(None),
    }
}

impl Writer<'_, '_> {
    async fn stop(
        &mut self,
        turn: &Row,
        requested: Option<turn::Cause>,
        occurred: &str,
    ) -> Result<Option<Row>, keel::adapt::Error> {
        let key = turn.key().to_string();
        if let Some(stop) = self
            .0
            .one(&form("TurnStop").when("turn", Op::Eq, &key))
            .await?
        {
            return Ok(Some(stop));
        }
        let Some(requested) = requested else {
            return Ok(None);
        };
        let key = self
            .0
            .put(
                "TurnStop",
                &[
                    ("cause", requested.encode()),
                    ("requested", occurred),
                    ("turn", &key),
                ],
            )
            .await?;
        self.0
            .one(&form("TurnStop").when("id", Op::Eq, &key.to_string()))
            .await
    }

    async fn settle(
        &mut self,
        stop: Option<&Row>,
        occurred: &str,
    ) -> Result<(), keel::adapt::Error> {
        if let Some(stop) = stop
            && stop.text("settled").is_none()
        {
            self.0
                .set("TurnStop", stop.key(), &[("settled", occurred)])
                .await?;
        }
        Ok(())
    }

    async fn notice(&mut self, notice: Notice<'_>) -> Result<String, keel::adapt::Error> {
        let tag = santi_model::tag("msg");
        let content = serde_json::to_string(&message::Content::text(format!(
        "<system_message>\nThe previous turn ({}) was interrupted by {}. Its partial output and external effects may be incomplete; inspect durable effect state before retrying.\n</system_message>",
        notice.turn,
        notice.cause.encode()
    )))
    .map_err(|error| keel::adapt::Error::Adapt(error.to_string()))?;
        self.0
            .put(
                "Message",
                &[
                    ("tag", tag.as_str()),
                    ("actor_type", "system"),
                    ("actor", notice.actor),
                    ("kind", "santi_system"),
                    ("content", content.as_str()),
                    ("state", "fixed"),
                    ("request", "false"),
                    ("created", notice.occurred),
                    ("updated", notice.occurred),
                ],
            )
            .await?;
        let strand = self
            .0
            .one(&form("Strand").when("id", Op::Eq, &notice.strand.key().to_string()))
            .await?
            .ok_or_else(|| keel::adapt::Error::Missing("interruption strand".into()))?;
        write::append(
            self.0,
            write::Entry {
                strand: &strand,
                kind: "message",
                target: &tag,
                created: notice.occurred,
            },
        )
        .await?;
        Ok(tag)
    }

    async fn related(
        &mut self,
        row: &Row,
        field: &str,
        missing: &str,
    ) -> Result<Row, keel::adapt::Error> {
        let key = row
            .int(field)
            .ok_or_else(|| keel::adapt::Error::Adapt(format!("{missing} missing")))?;
        self.0
            .one(&form("Strand").when("id", Op::Eq, &key.to_string()))
            .await?
            .ok_or_else(|| keel::adapt::Error::Missing(missing.into()))
    }
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
