use keel::adapt::db::Sqlite;
use keel::{Core, Op, Row, Tx, form};

pub(in crate::store) fn error(error: keel::adapt::Error) -> String {
    error.to_string()
}

pub(in crate::store) async fn one(
    core: &Core<Sqlite>,
    unit: &str,
    field: &str,
    value: &str,
) -> Result<Option<Row>, String> {
    core.one(&form(unit).when(field, Op::Eq, value))
        .await
        .map_err(error)
}

pub(in crate::store) async fn need(
    tx: &mut Tx<'_, Sqlite>,
    unit: &str,
    field: &str,
    value: &str,
) -> Result<i64, keel::adapt::Error> {
    tx.one(&form(unit).when(field, Op::Eq, value))
        .await?
        .map(|row| row.key())
        .ok_or_else(|| keel::adapt::Error::Missing(format!("{unit} {value}")))
}

pub(in crate::store) fn text<'a>(row: &'a Row, field: &str) -> Result<&'a str, String> {
    row.text(field)
        .ok_or_else(|| format!("{} missing {field}", row.key()))
}

pub(in crate::store) fn int(row: &Row, field: &str) -> Result<i64, String> {
    row.int(field)
        .ok_or_else(|| format!("{} missing {field}", row.key()))
}

pub(in crate::store) fn soul(row: &Row) -> Result<santi_model::soul::Soul, String> {
    Ok(santi_model::soul::Soul {
        id: text(row, "tag")?.to_string(),
        created: text(row, "created")?.to_string(),
        updated: text(row, "updated")?.to_string(),
    })
}

pub(in crate::store) async fn strand(
    core: &Core<Sqlite>,
    row: &Row,
) -> Result<santi_model::strand::Strand, String> {
    let soul = related(core, "Soul", int(row, "soul")?).await?;
    let parent = match row.int("parent") {
        Some(parent) => Some(related(core, "Strand", parent).await?),
        None => None,
    };
    let state = row
        .text("state")
        .map(serde_json::from_str)
        .transpose()
        .map_err(|error| error.to_string())?;
    Ok(santi_model::strand::Strand {
        id: text(row, "tag")?.to_string(),
        soul,
        label: row.text("label").map(str::to_string),
        memory: text(row, "memory")?.to_string(),
        state,
        next: int(row, "next")?,
        seen: int(row, "seen")?,
        parent,
        fork: row.int("fork"),
        created: text(row, "created")?.to_string(),
        updated: text(row, "updated")?.to_string(),
    })
}

pub(in crate::store) async fn related(
    core: &Core<Sqlite>,
    unit: &str,
    key: i64,
) -> Result<String, String> {
    one(core, unit, "id", &key.to_string())
        .await?
        .ok_or_else(|| format!("{unit} {key} missing"))
        .and_then(|row| text(&row, "tag").map(str::to_string))
}
