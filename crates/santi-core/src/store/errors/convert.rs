use rusqlite::Row;
use santi_error::{
    ErrorIncident, ErrorScope, ErrorSource, category_from_db, incident_status_from_db,
    retry_from_db, severity_from_db,
};

pub(super) fn map_incident_row(row: &Row<'_>) -> rusqlite::Result<ErrorIncident> {
    let exposure = parse_json(row.get::<_, String>(7)?, 7)?;
    let context = parse_json(row.get::<_, String>(16)?, 16)?;
    let latest_context = parse_json(row.get::<_, String>(17)?, 17)?;
    Ok(ErrorIncident {
        id: row.get(0)?,
        incident_key: row.get(1)?,
        code: row.get(2)?,
        status: incident_status_from_db(&row.get::<_, String>(3)?),
        category: category_from_db(&row.get::<_, String>(4)?),
        severity: severity_from_db(&row.get::<_, String>(5)?),
        retry: retry_from_db(&row.get::<_, String>(6)?),
        exposure,
        scope: ErrorScope::new(row.get::<_, String>(8)?, row.get::<_, String>(9)?),
        source: ErrorSource::new(row.get::<_, String>(10)?, row.get::<_, String>(11)?),
        latest_source: ErrorSource::new(row.get::<_, String>(12)?, row.get::<_, String>(13)?),
        message: row.get(14)?,
        latest_message: row.get(15)?,
        context,
        latest_context,
        occurrence_count: row.get(18)?,
        revision: row.get(19)?,
        first_seen_at: row.get(20)?,
        last_seen_at: row.get(21)?,
        resolved_at: row.get(22)?,
        resolved_by: row.get(23)?,
    })
}

pub(super) fn parse_json<T: serde::de::DeserializeOwned>(
    raw: String,
    index: usize,
) -> rusqlite::Result<T> {
    serde_json::from_str(&raw).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}
