use keel::Row;

pub(super) struct Origin<'a> {
    pub soul: &'a Row,
    pub strand: &'a Row,
    pub turn: &'a Row,
    pub call: &'a Row,
    pub effect: &'a Row,
}

pub(super) fn validate(origin: Origin<'_>) -> Result<(), keel::adapt::Error> {
    if origin.strand.int("soul") != Some(origin.soul.key())
        || origin.turn.int("strand") != Some(origin.strand.key())
        || origin.call.int("turn") != Some(origin.turn.key())
        || origin.effect.int("turn") != Some(origin.turn.key())
        || origin.effect.int("call") != Some(origin.call.key())
    {
        return Err(adapt("job capability origin is inconsistent"));
    }
    Ok(())
}

pub(super) fn signed(value: u64, label: &str) -> Result<String, keel::adapt::Error> {
    i64::try_from(value)
        .map(|value| value.to_string())
        .map_err(|_| adapt(&format!("{label} is out of range")))
}

pub(super) fn key(row: &Row, relation: &str) -> Result<String, keel::adapt::Error> {
    row.int(relation)
        .map(|key| key.to_string())
        .ok_or_else(|| adapt("job capability relation missing"))
}

pub(super) fn tag(row: &Row) -> Result<String, keel::adapt::Error> {
    row.text("tag")
        .map(str::to_string)
        .ok_or_else(|| adapt("job tag missing"))
}

pub(super) fn adapt(message: &str) -> keel::adapt::Error {
    keel::adapt::Error::Adapt(message.to_string())
}
