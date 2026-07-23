use rusqlite::{OptionalExtension, Row, params};
use santi_error::{Fault, Incident, Mutation, Report, Resolution, engine};

use super::Database;

const COLUMNS: &str = r#"
    id, incident_key, code, status, category, severity, retry, exposure,
    scope_kind, scope_id, source_component, source_operation,
    latest_source_component, latest_source_operation, message, latest_message,
    context, latest_context, occurrence_count, revision, first_seen_at,
    last_seen_at, resolved_at, resolved_by
"#;

impl Database<'_> {
    pub fn open(&self, draft: santi_error::Draft) -> Result<Fault, String> {
        let existing = self.incident(&draft.key)?;
        let mutation = engine().open(existing.as_ref(), draft, santi_model::now());
        self.persist(&mutation)?;
        Ok(mutation.error)
    }

    pub fn resolve(&self, key: &str, by: &str, context: serde_json::Value) -> Result<bool, String> {
        let Some(active) = self.incident(key)? else {
            return Ok(false);
        };
        let mutation = engine().resolve(&active, by, context, santi_model::now());
        self.persist(&mutation)?;
        Ok(true)
    }

    pub fn incident(&self, key: &str) -> Result<Option<Incident>, String> {
        self.conn
            .query_row(
                &format!(
                    "SELECT {COLUMNS} FROM error_incidents WHERE incident_key = ?1 AND status = 'active' LIMIT 1"
                ),
                params![key],
                incident,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub fn incidents(&self, kind: &str, id: &str, limit: i64) -> Result<Vec<Incident>, String> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT {COLUMNS} FROM error_incidents \
                 WHERE scope_kind = ?1 AND scope_id = ?2 \
                 ORDER BY first_seen_at DESC, id DESC LIMIT ?3"
            ))
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map(params![kind, id, limit.clamp(1, 1000)], incident)
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    pub fn persist(&self, mutation: &Mutation) -> Result<(), String> {
        let held = &mutation.incident;
        self.conn
            .execute(
                r#"
        INSERT INTO error_incidents (
          id, incident_key, code, status, category, severity, retry, exposure,
          scope_kind, scope_id, source_component, source_operation,
          latest_source_component, latest_source_operation, message, latest_message,
          context, latest_context, occurrence_count, revision, first_seen_at,
          last_seen_at, resolved_at, resolved_by
        ) VALUES (
          ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
          ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24
        )
        ON CONFLICT(id) DO UPDATE SET
          status = excluded.status,
          latest_source_component = excluded.latest_source_component,
          latest_source_operation = excluded.latest_source_operation,
          latest_message = excluded.latest_message,
          latest_context = excluded.latest_context,
          occurrence_count = excluded.occurrence_count,
          revision = excluded.revision,
          last_seen_at = excluded.last_seen_at,
          resolved_at = excluded.resolved_at,
          resolved_by = excluded.resolved_by
        "#,
                params![
                    held.id,
                    held.key,
                    held.code,
                    held.status.db(),
                    held.category.db(),
                    held.severity.db(),
                    held.retry.db(),
                    serde_json::to_string(&held.exposure).map_err(|error| error.to_string())?,
                    held.scope.kind,
                    held.scope.id,
                    held.first.source.component,
                    held.first.source.operation,
                    held.latest.source.component,
                    held.latest.source.operation,
                    held.first.message,
                    held.latest.message,
                    serde_json::to_string(&held.first.context).map_err(|error| error.to_string())?,
                    serde_json::to_string(&held.latest.context)
                        .map_err(|error| error.to_string())?,
                    held.occurrences,
                    held.revision,
                    held.first.seen,
                    held.latest.seen,
                    held.resolution.as_ref().map(|held| held.at.clone()),
                    held.resolution.as_ref().and_then(|held| held.by.clone()),
                ],
            )
            .map_err(|error| error.to_string())?;

        if let Some(transition) = mutation.transition.as_ref() {
            self.conn
                .execute(
                    r#"
            INSERT INTO error_transitions (
              id, incident_id, revision, kind, payload, created_at, delivered_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)
            "#,
                    params![
                        transition.id,
                        transition.incident,
                        transition.revision,
                        transition.kind.db(),
                        serde_json::to_string(transition).map_err(|error| error.to_string())?,
                        transition.occurred,
                    ],
                )
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }
}

fn incident(row: &Row<'_>) -> rusqlite::Result<Incident> {
    let exposure = parsed(row.get::<_, String>(7)?, 7)?;
    let context = parsed(row.get::<_, String>(16)?, 16)?;
    let latest = parsed(row.get::<_, String>(17)?, 17)?;
    let at: Option<String> = row.get(22)?;
    Ok(Incident {
        id: row.get(0)?,
        key: row.get(1)?,
        code: row.get(2)?,
        status: santi_error::Status::read(&row.get::<_, String>(3)?),
        category: santi_error::Category::read(&row.get::<_, String>(4)?),
        severity: santi_error::Severity::read(&row.get::<_, String>(5)?),
        retry: santi_error::Retry::read(&row.get::<_, String>(6)?),
        exposure,
        scope: santi_error::Scope::new(row.get::<_, String>(8)?, row.get::<_, String>(9)?),
        first: Report {
            source: santi_error::Source::new(row.get::<_, String>(10)?, row.get::<_, String>(11)?),
            message: row.get(14)?,
            context,
            seen: row.get(20)?,
        },
        latest: Report {
            source: santi_error::Source::new(row.get::<_, String>(12)?, row.get::<_, String>(13)?),
            message: row.get(15)?,
            context: latest,
            seen: row.get(21)?,
        },
        occurrences: row.get(18)?,
        revision: row.get(19)?,
        resolution: at
            .map(|at| {
                Ok::<_, rusqlite::Error>(Resolution {
                    at,
                    by: row.get(23)?,
                })
            })
            .transpose()?,
    })
}

fn parsed<T: serde::de::DeserializeOwned>(raw: String, index: usize) -> rusqlite::Result<T> {
    serde_json::from_str(&raw).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}
