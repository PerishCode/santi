use keel::Row;
use santi_error::{
    Category, Incident, Report, Resolution, Retry, Severity, Source, Status, Transition,
};

use crate::store::read;

pub(super) fn incident(row: &Row, resolved: bool) -> Result<Incident, String> {
    let resolution = if resolved {
        Some(Resolution {
            at: read::text(row, "resolved")?.to_string(),
            by: row.text("resolved_by").map(str::to_string),
        })
    } else {
        None
    };
    Ok(Incident {
        id: read::text(row, "tag")?.to_string(),
        key: read::text(row, "incident_key")?.to_string(),
        code: read::text(row, "code")?.to_string(),
        status: if resolved {
            Status::Resolved
        } else {
            Status::Active
        },
        category: category(read::text(row, "category")?)?,
        severity: severity(read::text(row, "severity")?)?,
        retry: retry(read::text(row, "retry")?)?,
        exposure: serde_json::from_str(read::text(row, "exposure")?)
            .map_err(|error| error.to_string())?,
        scope: santi_error::Scope::new(
            read::text(row, "scope_kind")?,
            read::text(row, "scope_id")?,
        ),
        first: report(row, false)?,
        latest: report(row, true)?,
        occurrences: read::int(row, "occurrence_count")?,
        revision: read::int(row, "revision")?,
        resolution,
    })
}

pub(super) fn transition(row: &Row) -> Result<Transition, String> {
    serde_json::from_str(read::text(row, "payload")?).map_err(|error| error.to_string())
}

fn report(row: &Row, latest: bool) -> Result<Report, String> {
    let prefix = if latest { "latest_" } else { "" };
    let source_component = format!("{prefix}source_component");
    let source_operation = format!("{prefix}source_operation");
    let message = format!("{prefix}message");
    let context = format!("{prefix}context");
    let seen = if latest { "last_seen" } else { "first_seen" };
    Ok(Report {
        source: Source::new(
            read::text(row, &source_component)?,
            read::text(row, &source_operation)?,
        ),
        message: read::text(row, &message)?.to_string(),
        context: serde_json::from_str(read::text(row, &context)?)
            .map_err(|error| error.to_string())?,
        seen: read::text(row, seen)?.to_string(),
    })
}

fn category(value: &str) -> Result<Category, String> {
    match value {
        "internal" => Ok(Category::Internal),
        "invalid" => Ok(Category::Invalid),
        "missing" => Ok(Category::Missing),
        "exhausted" => Ok(Category::Exhausted),
        "unauthorized" => Ok(Category::Unauthorized),
        "unavailable" => Ok(Category::Unavailable),
        value => Err(format!("unknown error category {value}")),
    }
}

fn severity(value: &str) -> Result<Severity, String> {
    match value {
        "error" => Ok(Severity::Error),
        value => Err(format!("unknown error severity {value}")),
    }
}

fn retry(value: &str) -> Result<Retry, String> {
    match value {
        "never" => Ok(Retry::Never),
        "later" => Ok(Retry::Later),
        "changed" => Ok(Retry::Changed),
        "resolved" => Ok(Retry::Resolved),
        value => Err(format!("unknown error retry {value}")),
    }
}
