use rusqlite::{Connection, params};
use serde_json::json;

use crate::store::{SantiStore, db::Database};
use crate::{Fault, catalog};

const DRIVE_DETAIL_BYTES: usize = 4096;

pub(crate) struct Input<'a> {
    pub operation: &'a str,
    pub trigger: &'a str,
    pub accepted_inbox_id: Option<&'a str>,
    pub detail: &'a str,
}

pub(crate) fn drive_incident_key(strand: &str) -> String {
    format!("{}:strand:{strand}", catalog::STRAND_DRIVE_FAILED.code)
}

pub(in crate::store) fn repeat_active_in_conn(
    conn: &Connection,
    strand: &str,
    operation: &str,
) -> Result<Option<Fault>, String> {
    let database = Database::new(conn);
    if database.incident(&drive_incident_key(strand))?.is_none() {
        return Ok(None);
    }
    let pending = pending_count(conn, strand)?;
    database
        .open(drive_draft(
            strand,
            Input {
                operation,
                trigger: "admission_guard",
                accepted_inbox_id: None,
                detail: "strand driver recovery is still required",
            },
            pending,
        ))
        .map(Some)
}

pub(in crate::store) fn resolve_in_conn(
    conn: &Connection,
    strand: &str,
    turn: &str,
    drained_count: usize,
) -> Result<Option<String>, String> {
    let database = Database::new(conn);
    let incident_id = database
        .incident(&drive_incident_key(strand))?
        .map(|incident| incident.id);
    database.resolve(
        &drive_incident_key(strand),
        "strand.drive_started",
        json!({
            "schema": "santi.error.strand_drive.resolution.v1",
            "turn": turn,
            "drained_count": drained_count,
        }),
    )?;
    Ok(incident_id)
}

impl SantiStore {
    pub(crate) fn reject_if_drive_blocked(&self, strand: &str) -> Result<Option<Fault>, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let error = repeat_active_in_conn(&tx, strand, "ingest_active_guard")?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(error)
    }

    pub(crate) fn record_drive_failure(
        &self,
        strand: &str,
        input: Input<'_>,
    ) -> Result<Fault, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let pending = pending_count(&tx, strand)?;
        let error = Database::new(&tx).open(drive_draft(strand, input, pending))?;
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

fn drive_draft(strand: &str, input: Input<'_>, pending: i64) -> santi_error::Draft {
    santi_error::Draft {
        key: drive_incident_key(strand),
        descriptor: catalog::STRAND_DRIVE_FAILED,
        scope: santi_error::Scope::new("strand", strand),
        source: santi_error::Source::new("santi-core", input.operation),
        message: "strand driver could not start pending work".to_string(),
        context: json!({
            "schema": "santi.error.strand_drive.v1",
            "accepted_before_failure": input.accepted_inbox_id.is_some(),
            "inbox": input.accepted_inbox_id,
            "pending_count": pending,
            "trigger": input.trigger,
            "detail": bounded_detail(input.detail),
            "recovery": {
                "command": format!("santi strand drive {strand}"),
                "resend": false,
            },
        }),
    }
}

fn pending_count(conn: &Connection, strand: &str) -> Result<i64, String> {
    conn.query_row(
        "SELECT COUNT(*) FROM strand_inbox WHERE strand_id = ?1",
        params![strand],
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
