use keel::Tx;
use keel::adapt::db::Sqlite;

pub(in crate::store) struct Entry<'a> {
    pub strand: &'a keel::Row,
    pub kind: &'a str,
    pub target: &'a str,
    pub created: &'a str,
}

impl<'a> Entry<'a> {
    pub fn new(strand: &'a keel::Row, kind: &'a str, target: &'a str, created: &'a str) -> Self {
        Self {
            strand,
            kind,
            target,
            created,
        }
    }
}

pub(in crate::store) async fn append(
    tx: &mut Tx<'_, Sqlite>,
    entry: Entry<'_>,
) -> Result<i64, keel::adapt::Error> {
    let sequence = entry
        .strand
        .int("next")
        .ok_or_else(|| keel::adapt::Error::Adapt("strand next missing".into()))?;
    tx.put(
        "StrandEntry",
        &[
            ("target_type", entry.kind),
            ("target", entry.target),
            ("sequence", &sequence.to_string()),
            ("created", entry.created),
            ("strand", &entry.strand.key().to_string()),
        ],
    )
    .await?;
    tx.set(
        "Strand",
        entry.strand.key(),
        &[
            ("next", &(sequence + 1).to_string()),
            ("updated", entry.created),
        ],
    )
    .await?;
    Ok(sequence)
}
