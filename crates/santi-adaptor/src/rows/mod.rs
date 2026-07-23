use rusqlite::Row;

mod decode;

pub trait Decode: Sized {
    fn decode(row: &Row<'_>) -> rusqlite::Result<Self>;
}

pub fn collected<T>(rows: impl Iterator<Item = rusqlite::Result<T>>) -> Result<Vec<T>, String> {
    let mut items = Vec::new();
    for row in rows {
        items.push(row.map_err(|error| error.to_string())?);
    }
    Ok(items)
}
