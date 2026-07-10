use rusqlite::{Connection, params};
use serde_json::json;

use super::{SantiStore, errors};
use crate::{ErrorScope, ErrorSource, IncidentDraft, SantiError, catalog};

const DRIVE_DETAIL_BYTES: usize = 4096;

pub(crate) struct DriveFailureInput<'a> {
    pub operation: &'a str,
    pub trigger_type: &'a str,
    pub accepted_inbox_id: Option<&'a str>,
    pub detail: &'a str,
}

pub(crate) fn drive_incident_key(strand_id: &str) -> String {
    format!("{}:strand:{strand_id}", catalog::STRAND_DRIVE_FAILED.code)
}

pub(super) fn repeat_active_in_conn(
    conn: &Connection,
    strand_id: &str,
    operation: &str,
) -> Result<Option<SantiError>, String> {
    if errors::active_in_conn(conn, &drive_incident_key(strand_id))?.is_none() {
        return Ok(None);
    }
    let pending = pending_count(conn, strand_id)?;
    errors::open_incident_in_conn(
        conn,
        drive_draft(
            strand_id,
            DriveFailureInput {
                operation,
                trigger_type: "admission_guard",
                accepted_inbox_id: None,
                detail: "strand driver recovery is still required",
            },
            pending,
        ),
    )
    .map(Some)
}

pub(super) fn resolve_in_conn(
    conn: &Connection,
    strand_id: &str,
    turn_id: &str,
    drained_count: usize,
) -> Result<bool, String> {
    errors::resolve_in_conn(
        conn,
        &drive_incident_key(strand_id),
        "strand.drive_started",
        json!({
            "schema": "santi.error.strand_drive.resolution.v1",
            "turn_id": turn_id,
            "drained_count": drained_count,
        }),
    )
}

impl SantiStore {
    pub(crate) fn reject_if_drive_blocked(
        &self,
        strand_id: &str,
    ) -> Result<Option<SantiError>, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let error = repeat_active_in_conn(&tx, strand_id, "ingest_active_guard")?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(error)
    }

    pub(crate) fn record_drive_failure(
        &self,
        strand_id: &str,
        input: DriveFailureInput<'_>,
    ) -> Result<SantiError, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let pending = pending_count(&tx, strand_id)?;
        let error = errors::open_incident_in_conn(&tx, drive_draft(strand_id, input, pending))?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(error)
    }

    pub(crate) fn active_drive_incident_count(&self) -> Result<i64, String> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM error_incidents WHERE code = ?1 AND status = 'active'",
            params![catalog::STRAND_DRIVE_FAILED.code],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())
    }
}

fn drive_draft(strand_id: &str, input: DriveFailureInput<'_>, pending: i64) -> IncidentDraft {
    IncidentDraft {
        incident_key: drive_incident_key(strand_id),
        descriptor: catalog::STRAND_DRIVE_FAILED,
        scope: ErrorScope::new("strand", strand_id),
        source: ErrorSource::new("santi-core", input.operation),
        message: "strand driver could not start pending work".to_string(),
        context: json!({
            "schema": "santi.error.strand_drive.v1",
            "accepted_before_failure": input.accepted_inbox_id.is_some(),
            "inbox_id": input.accepted_inbox_id,
            "pending_count": pending,
            "trigger_type": input.trigger_type,
            "detail": bounded_detail(input.detail),
            "recovery": {
                "command": format!("santi strand drive {strand_id}"),
                "resend": false,
            },
        }),
    }
}

fn pending_count(conn: &Connection, strand_id: &str) -> Result<i64, String> {
    conn.query_row(
        "SELECT COUNT(*) FROM strand_inbox WHERE strand_id = ?1",
        params![strand_id],
        |row| row.get(0),
    )
    .map_err(|error| error.to_string())
}

fn bounded_detail(detail: &str) -> String {
    if detail.len() <= DRIVE_DETAIL_BYTES {
        return detail.to_string();
    }
    let suffix = " [truncated]";
    let mut end = DRIVE_DETAIL_BYTES.saturating_sub(suffix.len());
    while end > 0 && !detail.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{}", &detail[..end], suffix)
}
