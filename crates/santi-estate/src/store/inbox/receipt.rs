use super::ReceiptDraft;
use crate::store::read;
use keel::adapt::db::Sqlite;
use keel::{Core, Op, Rank, Tx, form};
use santi_model::receipt;
use std::collections::BTreeSet;

struct Draft<'a> {
    receipt: i64,
    sequence: i64,
    state: receipt::State,
    turn: Option<i64>,
    incident: Option<&'a str>,
    rebuilt: Option<&'a str>,
    occurred: &'a str,
}

pub(in crate::store) struct Close<'a> {
    pub turn: &'a str,
    pub state: receipt::State,
    pub incident: Option<&'a str>,
    pub occurred: &'a str,
}

struct Writer<'a, 'tx>(&'a mut Tx<'tx, Sqlite>);

pub(in crate::store) async fn accept(
    tx: &mut Tx<'_, Sqlite>,
    inbox: &str,
    strand: &str,
    accepted: &str,
) -> Result<(), keel::adapt::Error> {
    let receipt = tx
        .put(
            "InboxReceipt",
            &[
                ("tag", inbox),
                ("accepted", accepted),
                ("updated", accepted),
                ("strand", strand),
            ],
        )
        .await?;
    Writer(tx)
        .transition(Draft {
            receipt,
            sequence: 1,
            state: receipt::State::Accepted,
            turn: None,
            incident: None,
            rebuilt: None,
            occurred: accepted,
        })
        .await
}

pub(super) async fn advance(core: &Core<Sqlite>, draft: ReceiptDraft<'_>) -> Result<(), String> {
    core.batch(async |tx| shift(tx, draft).await)
        .await
        .map_err(read::error)
}

pub(in crate::store) async fn shift(
    tx: &mut Tx<'_, Sqlite>,
    draft: ReceiptDraft<'_>,
) -> Result<(), keel::adapt::Error> {
    let receipt = tx
        .one(&form("InboxReceipt").when("tag", Op::Eq, draft.inbox))
        .await?
        .ok_or_else(|| keel::adapt::Error::Missing(draft.inbox.into()))?;
    let last = tx
        .one(
            &form("ReceiptTransition")
                .when("receipt", Op::Eq, &receipt.key().to_string())
                .order("sequence", Rank::Desc)
                .top(1),
        )
        .await?
        .ok_or_else(|| keel::adapt::Error::Adapt("receipt transition missing".into()))?;
    let sequence = last
        .int("sequence")
        .ok_or_else(|| keel::adapt::Error::Adapt("receipt sequence missing".into()))?
        + 1;
    let turn = match draft.turn {
        Some(turn) => Some(read::need(tx, "Turn", "tag", turn).await?),
        None => None,
    };
    tx.set(
        "InboxReceipt",
        receipt.key(),
        &[
            ("state", state_text(&draft.state)),
            ("updated", draft.occurred),
        ],
    )
    .await?;
    Writer(tx)
        .transition(Draft {
            receipt: receipt.key(),
            sequence,
            state: draft.state,
            turn,
            incident: draft.incident,
            rebuilt: draft.rebuilt,
            occurred: draft.occurred,
        })
        .await
}

pub(in crate::store) async fn close(
    tx: &mut Tx<'_, Sqlite>,
    draft: Close<'_>,
) -> Result<(), keel::adapt::Error> {
    let turn_row = read::need(tx, "Turn", "tag", draft.turn).await?;
    let transitions = tx
        .ask(
            &form("ReceiptTransition")
                .when("turn", Op::Eq, &turn_row.to_string())
                .when("state", Op::Eq, "driving"),
        )
        .await?;
    let receipts = transitions
        .rows()
        .iter()
        .filter_map(|transition| transition.int("receipt"))
        .collect::<BTreeSet<_>>();
    for key in receipts {
        let receipt_row = tx
            .one(&form("InboxReceipt").when("id", Op::Eq, &key.to_string()))
            .await?
            .ok_or_else(|| keel::adapt::Error::Missing("driving receipt".into()))?;
        if !matches!(receipt_row.text("state"), Some("driving" | "recovered")) {
            continue;
        }
        let tag = receipt_row
            .text("tag")
            .ok_or_else(|| keel::adapt::Error::Adapt("receipt tag missing".into()))?
            .to_string();
        shift(
            tx,
            ReceiptDraft {
                inbox: &tag,
                state: draft.state.clone(),
                turn: Some(draft.turn),
                incident: draft.incident,
                rebuilt: None,
                occurred: draft.occurred,
            },
        )
        .await?;
    }
    Ok(())
}

pub(super) async fn status(
    core: &Core<Sqlite>,
    inbox: &str,
) -> Result<Option<receipt::Status>, String> {
    let Some(row) = read::one(core, "InboxReceipt", "tag", inbox).await? else {
        return Ok(None);
    };
    let transitions = core
        .ask(
            &form("ReceiptTransition")
                .when("receipt", Op::Eq, &row.key().to_string())
                .order("sequence", Rank::Asc),
        )
        .await
        .map_err(read::error)?;
    let mut history = Vec::with_capacity(transitions.rows().len());
    for transition in transitions.rows() {
        history.push(decode_transition(core, transition).await?);
    }
    Ok(Some(receipt::Status {
        inbox: read::text(&row, "tag")?.to_string(),
        strand: read::related(core, "Strand", read::int(&row, "strand")?).await?,
        state: decode_state(read::text(&row, "state")?)?,
        accepted: read::text(&row, "accepted")?.to_string(),
        updated: read::text(&row, "updated")?.to_string(),
        transitions: history,
        effects: Vec::new(),
    }))
}

impl Writer<'_, '_> {
    async fn transition(&mut self, draft: Draft<'_>) -> Result<(), keel::adapt::Error> {
        let tag = santi_model::tag("rct");
        let receipt = draft.receipt.to_string();
        let sequence = draft.sequence.to_string();
        let turn = draft.turn.map(|turn| turn.to_string());
        let mut fields = vec![
            ("tag", tag.as_str()),
            ("sequence", sequence.as_str()),
            ("state", state_text(&draft.state)),
            ("occurred", draft.occurred),
            ("receipt", receipt.as_str()),
        ];
        if let Some(turn) = turn.as_deref() {
            fields.push(("turn", turn));
        }
        if let Some(incident) = draft.incident {
            fields.push(("incident", incident));
        }
        if let Some(rebuilt) = draft.rebuilt {
            fields.push(("rebuilt", rebuilt));
        }
        self.0.put("ReceiptTransition", &fields).await?;
        Ok(())
    }
}

async fn decode_transition(
    core: &Core<Sqlite>,
    row: &keel::Row,
) -> Result<receipt::Transition, String> {
    let turn = match row.int("turn") {
        Some(turn) => Some(read::related(core, "Turn", turn).await?),
        None => None,
    };
    Ok(receipt::Transition {
        id: read::text(row, "tag")?.to_string(),
        sequence: read::int(row, "sequence")?,
        state: decode_state(read::text(row, "state")?)?,
        turn,
        incident: row.text("incident").map(str::to_string),
        rebuilt: row.text("rebuilt").map(str::to_string),
        occurred: read::text(row, "occurred")?.to_string(),
    })
}

fn state_text(state: &receipt::State) -> &'static str {
    match state {
        receipt::State::Accepted => "accepted",
        receipt::State::Recovered => "recovered",
        receipt::State::Driving => "driving",
        receipt::State::Failed => "failed",
        receipt::State::Completed => "completed",
    }
}

fn decode_state(value: &str) -> Result<receipt::State, String> {
    match value {
        "accepted" => Ok(receipt::State::Accepted),
        "recovered" => Ok(receipt::State::Recovered),
        "driving" => Ok(receipt::State::Driving),
        "failed" => Ok(receipt::State::Failed),
        "completed" => Ok(receipt::State::Completed),
        value => Err(format!("unknown receipt state {value}")),
    }
}
