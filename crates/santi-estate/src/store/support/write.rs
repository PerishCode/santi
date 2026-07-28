use keel::Tx;
use keel::adapt::db::Sqlite;

pub(in crate::store) async fn append(
    tx: &mut Tx<'_, Sqlite>,
    strand: &keel::Row,
    kind: &str,
    target: &str,
    created: &str,
) -> Result<i64, keel::adapt::Error> {
    let sequence = strand
        .int("next")
        .ok_or_else(|| keel::adapt::Error::Adapt("strand next missing".into()))?;
    tx.put(
        "StrandEntry",
        &[
            ("target_type", kind),
            ("target", target),
            ("sequence", &sequence.to_string()),
            ("created", created),
            ("strand", &strand.key().to_string()),
        ],
    )
    .await?;
    tx.set(
        "Strand",
        strand.key(),
        &[("next", &(sequence + 1).to_string()), ("updated", created)],
    )
    .await?;
    Ok(sequence)
}
