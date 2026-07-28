use super::{Store, read};
use keel::{Op, Rank, form};
use santi_model::effect;
use std::collections::BTreeSet;

pub(super) async fn status(store: &Store, tag: &str) -> Result<Option<effect::Status>, String> {
    let Some(row) = read::one(&store.core, "StrandEffect", "tag", tag).await? else {
        return Ok(None);
    };
    Ok(Some(effect::Status {
        effect: decode(&store.core, &row).await?,
        receipts: receipts(&store.core, read::int(&row, "turn")?).await?,
    }))
}

impl Store {
    pub async fn effects(&self, strand: &str) -> Result<Vec<effect::Effect>, String> {
        let strand = read::one(&self.core, "Strand", "tag", strand)
            .await?
            .ok_or_else(|| "strand not found".to_string())?;
        let turns = self
            .core
            .ask(&form("Turn").when("strand", Op::Eq, &strand.key().to_string()))
            .await
            .map_err(read::error)?;
        let mut rows = Vec::new();
        for turn in turns.rows() {
            rows.extend(
                self.core
                    .ask(&form("StrandEffect").when("turn", Op::Eq, &turn.key().to_string()))
                    .await
                    .map_err(read::error)?
                    .rows()
                    .iter()
                    .cloned(),
            );
        }
        rows.sort_by(|left, right| {
            left.text("created")
                .cmp(&right.text("created"))
                .then_with(|| left.text("tag").cmp(&right.text("tag")))
        });
        let mut effects = Vec::with_capacity(rows.len());
        for row in &rows {
            effects.push(decode(&self.core, row).await?);
        }
        Ok(effects)
    }

    pub(crate) async fn effects_for_receipt(
        &self,
        inbox: &str,
    ) -> Result<Vec<effect::Effect>, String> {
        let Some(receipt) = read::one(&self.core, "InboxReceipt", "tag", inbox).await? else {
            return Ok(Vec::new());
        };
        let transitions = self
            .core
            .ask(
                &form("ReceiptTransition")
                    .when("receipt", Op::Eq, &receipt.key().to_string())
                    .order("sequence", Rank::Asc),
            )
            .await
            .map_err(read::error)?;
        let turns = transitions
            .rows()
            .iter()
            .filter_map(|row| row.int("turn"))
            .collect::<BTreeSet<_>>();
        let mut rows = Vec::new();
        for turn in turns {
            rows.extend(
                self.core
                    .ask(&form("StrandEffect").when("turn", Op::Eq, &turn.to_string()))
                    .await
                    .map_err(read::error)?
                    .rows()
                    .iter()
                    .cloned(),
            );
        }
        rows.sort_by(|left, right| {
            left.text("created")
                .cmp(&right.text("created"))
                .then_with(|| left.text("tag").cmp(&right.text("tag")))
        });
        rows.dedup_by_key(|row| row.key());
        let mut effects = Vec::with_capacity(rows.len());
        for row in &rows {
            effects.push(decode(&self.core, row).await?);
        }
        Ok(effects)
    }
}

async fn decode(
    core: &keel::Core<keel::adapt::db::Sqlite>,
    row: &keel::Row,
) -> Result<effect::Effect, String> {
    let turn_key = read::int(row, "turn")?;
    let turn = read::one(core, "Turn", "id", &turn_key.to_string())
        .await?
        .ok_or_else(|| "effect turn missing".to_string())?;
    let call = match row.int("call") {
        Some(call) => Some(read::related(core, "ToolCall", call).await?),
        None => None,
    };
    Ok(effect::Effect {
        id: read::text(row, "tag")?.to_string(),
        strand: read::related(core, "Strand", read::int(&turn, "strand")?).await?,
        turn: read::text(&turn, "tag")?.to_string(),
        call,
        kind: read::text(row, "effect_type")?.to_string(),
        state: super::write::decode_state(read::text(row, "state")?)?,
        result: row.text("result").map(str::to_string),
        error: row.text("error").map(str::to_string),
        created: read::text(row, "created")?.to_string(),
        updated: read::text(row, "updated")?.to_string(),
        dispatched: row.text("dispatched").map(str::to_string),
        settled: row.text("settled").map(str::to_string),
    })
}

async fn receipts(
    core: &keel::Core<keel::adapt::db::Sqlite>,
    turn: i64,
) -> Result<Vec<String>, String> {
    let transitions = core
        .ask(&form("ReceiptTransition").when("turn", Op::Eq, &turn.to_string()))
        .await
        .map_err(read::error)?;
    let mut receipts = BTreeSet::new();
    for transition in transitions.rows() {
        receipts
            .insert(read::related(core, "InboxReceipt", read::int(transition, "receipt")?).await?);
    }
    Ok(receipts.into_iter().collect())
}
