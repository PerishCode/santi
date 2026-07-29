use keel::adapt::db::Sqlite;
use keel::{Op, Row, Tx, form};
use santi_model::compact;

pub(super) struct Plan {
    pub(super) strand: i64,
    pub(super) first: i64,
    pub(super) last: i64,
    pub(super) first_tag: String,
    pub(super) last_tag: String,
    pub(super) from: i64,
    pub(super) to: i64,
    pub(super) absorbed: Vec<(i64, String)>,
    pub(super) collapsed: i64,
}

struct Reader<'a, 'tx>(&'a mut Tx<'tx, Sqlite>);

impl Plan {
    pub(super) fn report(&self, tag: &str, dry: bool) -> compact::Report {
        compact::Report {
            compact: tag.to_string(),
            first: self.first_tag.clone(),
            last: self.last_tag.clone(),
            from: self.from,
            to: self.to,
            absorbed: self.absorbed.iter().map(|(_, tag)| tag.clone()).collect(),
            collapsed: self.collapsed,
            dry,
            active_incident_resolved: false,
            before: None,
            after: None,
            ratio: None,
        }
    }
}

pub(super) async fn build(
    tx: &mut Tx<'_, Sqlite>,
    strand: &str,
    first: &str,
    last: &str,
) -> Result<Plan, keel::adapt::Error> {
    let strand = Reader(tx).need("Strand", strand).await?;
    let (first_row, from) = boundary(tx, strand.key(), first, "from").await?;
    let (last_row, to) = boundary(tx, strand.key(), last, "to").await?;
    if from > to {
        return Err(keel::adapt::Error::Adapt(
            "compact from must not be after to".into(),
        ));
    }
    let mut absorbed = Vec::new();
    let rows = tx
        .ask(&form("Compact").when("strand", Op::Eq, &strand.key().to_string()))
        .await?
        .rows()
        .to_vec();
    for compact in rows {
        let start = compact
            .int("first")
            .ok_or_else(|| keel::adapt::Error::Adapt("compact first missing".into()))?;
        let end = compact
            .int("last")
            .ok_or_else(|| keel::adapt::Error::Adapt("compact last missing".into()))?;
        let start = sequence(tx, strand.key(), start).await?;
        let end = sequence(tx, strand.key(), end).await?;
        if end < from || start > to {
            continue;
        }
        if from <= start && end <= to {
            absorbed.push((compact.key(), text(&compact, "tag")?.to_string()));
            continue;
        }
        return Err(keel::adapt::Error::Adapt(
            "compact range partially overlaps an existing compact".into(),
        ));
    }
    let collapsed = tx
        .ask(
            &form("StrandEntry")
                .when("strand", Op::Eq, &strand.key().to_string())
                .when("sequence", Op::Ge, &from.to_string())
                .when("sequence", Op::Le, &to.to_string()),
        )
        .await?
        .rows()
        .len() as i64;
    Ok(Plan {
        strand: strand.key(),
        first: first_row.key(),
        last: last_row.key(),
        first_tag: text(&first_row, "tag")?.to_string(),
        last_tag: text(&last_row, "tag")?.to_string(),
        from,
        to,
        absorbed,
        collapsed,
    })
}

async fn boundary(
    tx: &mut Tx<'_, Sqlite>,
    strand: i64,
    tag: &str,
    label: &str,
) -> Result<(Row, i64), keel::adapt::Error> {
    let message = Reader(tx).need("Message", tag).await?;
    if message.text("state") != Some("fixed")
        || !matches!(message.text("actor_type"), Some("soul" | "system"))
        || !matches!(message.text("kind"), Some("text" | "santi_system"))
    {
        return Err(keel::adapt::Error::Adapt(format!(
            "compact {label} boundary must be a fixed projected message"
        )));
    }
    let entry = tx
        .one(
            &form("StrandEntry")
                .when("strand", Op::Eq, &strand.to_string())
                .when("target_type", Op::Eq, "message")
                .when("target", Op::Eq, tag),
        )
        .await?
        .ok_or_else(|| {
            keel::adapt::Error::Adapt(format!("compact {label} message not in this strand"))
        })?;
    let sequence = entry
        .int("sequence")
        .ok_or_else(|| keel::adapt::Error::Adapt("compact sequence missing".into()))?;
    Ok((message, sequence))
}

pub(super) async fn sequence(
    tx: &mut Tx<'_, Sqlite>,
    strand: i64,
    message: i64,
) -> Result<i64, keel::adapt::Error> {
    let message = tx
        .one(&form("Message").when("id", Op::Eq, &message.to_string()))
        .await?
        .ok_or_else(|| keel::adapt::Error::Missing("compact boundary message".into()))?;
    let tag = text(&message, "tag")?;
    tx.one(
        &form("StrandEntry")
            .when("strand", Op::Eq, &strand.to_string())
            .when("target_type", Op::Eq, "message")
            .when("target", Op::Eq, tag),
    )
    .await?
    .and_then(|row| row.int("sequence"))
    .ok_or_else(|| keel::adapt::Error::Missing("compact boundary entry".into()))
}

impl Reader<'_, '_> {
    async fn need(&mut self, unit: &str, tag: &str) -> Result<Row, keel::adapt::Error> {
        self.0
            .one(&form(unit).when("tag", Op::Eq, tag))
            .await?
            .ok_or_else(|| keel::adapt::Error::Missing(format!("{unit} {tag}")))
    }
}

fn text<'a>(row: &'a Row, field: &str) -> Result<&'a str, keel::adapt::Error> {
    row.text(field)
        .ok_or_else(|| keel::adapt::Error::Adapt(format!("compact {field} missing")))
}
