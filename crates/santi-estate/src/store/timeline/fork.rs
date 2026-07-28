use super::{Store, read};

use keel::adapt::db::Sqlite;
use keel::{Op, Rank, Row, Tx, form};

#[derive(Clone, Copy)]
pub struct ForkDraft<'a> {
    pub tag: &'a str,
    pub parent: &'a str,
    pub at: i64,
    pub created: &'a str,
}

impl Store {
    pub async fn fork(&self, draft: ForkDraft<'_>) -> Result<santi_model::strand::Strand, String> {
        if draft.at < 0 {
            return Err("fork must be >= 0".to_string());
        }
        self.core
            .batch(async |tx| create(tx, draft).await)
            .await
            .map_err(read::error)?;
        self.strand(draft.tag)
            .await?
            .ok_or_else(|| "forked strand missing".to_string())
    }

    pub async fn discard_fork(&self, tag: &str) -> Result<bool, String> {
        self.core
            .batch(async |tx| discard(tx, tag).await)
            .await
            .map_err(read::error)
    }
}

async fn create(tx: &mut Tx<'_, Sqlite>, draft: ForkDraft<'_>) -> Result<(), keel::adapt::Error> {
    let parent = need(tx, "Strand", draft.parent).await?;
    let inherited = int(&parent, "next")?.saturating_sub(1);
    if draft.at > inherited {
        return Err(keel::adapt::Error::Adapt(format!(
            "fork {} is past parent end {inherited}",
            draft.at
        )));
    }
    let child = tx
        .put(
            "Strand",
            &[
                ("tag", draft.tag),
                ("next", &(draft.at + 1).to_string()),
                ("seen", &int(&parent, "seen")?.min(draft.at).to_string()),
                ("fork", &draft.at.to_string()),
                ("created", draft.created),
                ("updated", draft.created),
                ("soul", &int(&parent, "soul")?.to_string()),
                ("parent", &parent.key().to_string()),
            ],
        )
        .await?;
    copy_entries(tx, parent.key(), child, draft.at).await?;
    copy_compacts(tx, parent.key(), child, draft.at).await
}

async fn copy_entries(
    tx: &mut Tx<'_, Sqlite>,
    parent: i64,
    child: i64,
    at: i64,
) -> Result<(), keel::adapt::Error> {
    let entries = tx
        .ask(
            &form("StrandEntry")
                .when("strand", Op::Eq, &parent.to_string())
                .when("sequence", Op::Le, &at.to_string())
                .order("sequence", Rank::Asc),
        )
        .await?
        .rows()
        .to_vec();
    for entry in entries {
        tx.put(
            "StrandEntry",
            &[
                ("target_type", text(&entry, "target_type")?),
                ("target", text(&entry, "target")?),
                ("sequence", &int(&entry, "sequence")?.to_string()),
                ("created", text(&entry, "created")?),
                ("strand", &child.to_string()),
            ],
        )
        .await?;
    }
    Ok(())
}

async fn copy_compacts(
    tx: &mut Tx<'_, Sqlite>,
    parent: i64,
    child: i64,
    at: i64,
) -> Result<(), keel::adapt::Error> {
    let compacts = tx
        .ask(
            &form("Compact")
                .when("strand", Op::Eq, &parent.to_string())
                .order("created", Rank::Asc)
                .order("tag", Rank::Asc),
        )
        .await?
        .rows()
        .to_vec();
    for compact in compacts {
        let first = int(&compact, "first")?;
        let last = int(&compact, "last")?;
        let from = message_sequence(tx, parent, first).await?;
        let to = message_sequence(tx, parent, last).await?;
        if from > at || to > at {
            continue;
        }
        let tag = santi_model::tag("cmp");
        let first = first.to_string();
        let last = last.to_string();
        let child = child.to_string();
        let mut fields = vec![
            ("tag", tag.as_str()),
            ("summary", text(&compact, "summary")?),
            ("strand", child.as_str()),
            ("first", first.as_str()),
            ("last", last.as_str()),
        ];
        if let Some(created) = compact.text("created") {
            fields.push(("created", created));
        }
        if let Some(metadata) = compact.text("metadata") {
            fields.push(("metadata", metadata));
        }
        tx.put("Compact", &fields).await?;
    }
    Ok(())
}

async fn discard(tx: &mut Tx<'_, Sqlite>, tag: &str) -> Result<bool, keel::adapt::Error> {
    let Some(child) = tx.one(&form("Strand").when("tag", Op::Eq, tag)).await? else {
        return Ok(false);
    };
    if child.int("parent").is_none() {
        return Err(keel::adapt::Error::Adapt(
            "refusing to discard a non-fork strand".into(),
        ));
    }
    let fork = int(&child, "fork")?;
    if int(&child, "next")? != fork + 1 {
        return Err(keel::adapt::Error::Adapt(
            "refusing to discard a fork with new timeline entries".into(),
        ));
    }
    for unit in [
        "Turn",
        "StrandInbox",
        "InboxSlot",
        "InboxReceipt",
        "Job",
        "JobCapability",
    ] {
        if tx
            .one(&form(unit).when("strand", Op::Eq, &child.key().to_string()))
            .await?
            .is_some()
        {
            return Err(keel::adapt::Error::Adapt(format!(
                "refusing to discard a fork with live {unit}"
            )));
        }
    }
    end_all(tx, "Compact", child.key()).await?;
    end_all(tx, "StrandEntry", child.key()).await?;
    tx.end("Strand", child.key()).await?;
    Ok(true)
}

async fn end_all(
    tx: &mut Tx<'_, Sqlite>,
    unit: &str,
    strand: i64,
) -> Result<(), keel::adapt::Error> {
    let rows = tx
        .ask(&form(unit).when("strand", Op::Eq, &strand.to_string()))
        .await?
        .rows()
        .to_vec();
    for row in rows {
        tx.end(unit, row.key()).await?;
    }
    Ok(())
}

async fn message_sequence(
    tx: &mut Tx<'_, Sqlite>,
    strand: i64,
    message: i64,
) -> Result<i64, keel::adapt::Error> {
    let message = tx
        .one(&form("Message").when("id", Op::Eq, &message.to_string()))
        .await?
        .ok_or_else(|| keel::adapt::Error::Missing("compact boundary message".into()))?;
    tx.one(
        &form("StrandEntry")
            .when("strand", Op::Eq, &strand.to_string())
            .when("target_type", Op::Eq, "message")
            .when("target", Op::Eq, text(&message, "tag")?),
    )
    .await?
    .and_then(|row| row.int("sequence"))
    .ok_or_else(|| keel::adapt::Error::Missing("compact boundary entry".into()))
}

async fn need(tx: &mut Tx<'_, Sqlite>, unit: &str, tag: &str) -> Result<Row, keel::adapt::Error> {
    tx.one(&form(unit).when("tag", Op::Eq, tag))
        .await?
        .ok_or_else(|| keel::adapt::Error::Missing(format!("{unit} {tag}")))
}

fn text<'a>(row: &'a Row, field: &str) -> Result<&'a str, keel::adapt::Error> {
    row.text(field)
        .ok_or_else(|| keel::adapt::Error::Adapt(format!("fork {field} missing")))
}

fn int(row: &Row, field: &str) -> Result<i64, keel::adapt::Error> {
    row.int(field)
        .ok_or_else(|| keel::adapt::Error::Adapt(format!("fork {field} missing")))
}
