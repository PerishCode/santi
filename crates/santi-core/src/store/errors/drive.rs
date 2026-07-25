use crate::Ruled;
use rusqlite::{Connection, params};
use serde_json::json;

use crate::Fault;
use crate::store::{Store, db::Database};

const DETAIL: usize = 4096;

pub(crate) struct Input<'a> {
    pub operation: &'a str,
    pub trigger: &'a str,
    pub inbox: Option<&'a str>,
    pub detail: &'a str,
}

pub(in crate::store) fn stalled(
    conn: &Connection,
    strand: &str,
    operation: &str,
) -> Result<Option<Fault>, String> {
    let database = Database::new(conn);
    if database
        .incident(
            &crate::drive::Error::Failed
                .descriptor()
                .key("strand", strand),
        )?
        .is_none()
    {
        return Ok(None);
    }
    let pending = pending(conn, strand)?;
    database
        .open(draft(
            strand,
            Input {
                operation,
                trigger: "admission_guard",
                inbox: None,
                detail: "strand driver recovery is still required",
            },
            pending,
        ))
        .map(Some)
}

pub(in crate::store) fn revive(
    conn: &Connection,
    strand: &str,
    turn: &str,
    drained_count: usize,
) -> Result<Option<String>, String> {
    let database = Database::new(conn);
    let incident = database
        .incident(
            &crate::drive::Error::Failed
                .descriptor()
                .key("strand", strand),
        )?
        .map(|incident| incident.id);
    database.resolve(
        &crate::drive::Error::Failed
            .descriptor()
            .key("strand", strand),
        "strand.drive_started",
        json!({
            "schema": "santi.error.strand_drive.resolution.v1",
            "turn": turn,
            "drained_count": drained_count,
        }),
    )?;
    Ok(incident)
}

impl Store {
    pub(crate) fn gated(&self, strand: &str) -> Result<Option<Fault>, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let error = stalled(&tx, strand, "ingest_active_guard")?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(error)
    }

    pub(crate) fn stumbled(&self, strand: &str, input: Input<'_>) -> Result<Fault, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let pending = pending(&tx, strand)?;
        let error = Database::new(&tx).open(draft(strand, input, pending))?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(error)
    }

    pub(crate) fn strained(&self) -> Result<i64, String> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM error_incidents WHERE code = ?1 AND status = 'active'",
            params![crate::drive::Error::Failed.descriptor().code],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())
    }
}

fn draft(strand: &str, input: Input<'_>, pending: i64) -> santi_error::Draft {
    santi_error::Draft {
        key: crate::drive::Error::Failed
            .descriptor()
            .key("strand", strand),
        descriptor: crate::drive::Error::Failed.descriptor(),
        scope: santi_error::Scope::new("strand", strand),
        source: santi_error::Source::new("santi-core", input.operation),
        message: "strand driver could not start pending work".to_string(),
        context: json!({
            "schema": "santi.error.strand_drive.v1",
            "accepted_before_failure": input.inbox.is_some(),
            "inbox": input.inbox,
            "pending_count": pending,
            "trigger": input.trigger,
            "detail": bounded(input.detail),
            "recovery": {
                "command": format!("santi strand drive {strand}"),
                "resend": false,
            },
        }),
    }
}

fn pending(conn: &Connection, strand: &str) -> Result<i64, String> {
    conn.query_row(
        "SELECT COUNT(*) FROM strand_inbox WHERE strand_id = ?1",
        params![strand],
        |row| row.get(0),
    )
    .map_err(|error| error.to_string())
}

fn bounded(detail: &str) -> String {
    if detail.len() <= DETAIL {
        return detail.to_string();
    }
    let suffix = " [truncated]";
    let mut end = DETAIL.saturating_sub(suffix.len());
    while end > 0 && !detail.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{}", &detail[..end], suffix)
}
