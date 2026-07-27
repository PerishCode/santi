use rusqlite::{OptionalExtension, params};
use std::collections::BTreeSet;

use super::{GATE, Notice, Offered};
use crate::store::db::Database;
use crate::{now, tag};

pub(in crate::store) fn stow(
    tx: &rusqlite::Connection,
    notice: Notice<'_>,
) -> Result<Offered, String> {
    let held = tx
        .query_row(
            r#"
                SELECT revision, digest, inbox_id
                FROM inbox_slots
                WHERE strand_id = ?1 AND slot_key = ?2
                "#,
            params![notice.strand, notice.key],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let revision = i64::try_from(notice.revision)
        .map_err(|_| "inbox notice revision is out of range".to_string())?;
    if let Some((current, digest, inbox)) = &held {
        if revision < *current {
            return Ok(Offered {
                inbox: inbox.clone(),
                inserted: false,
            });
        }
        if revision == *current {
            if digest != notice.digest {
                return Err("inbox notice revision conflicts with its accepted payload".into());
            }
            return Ok(Offered {
                inbox: inbox.clone(),
                inserted: false,
            });
        }
    }

    let blob = serde_json::to_string(&notice.content).map_err(|error| error.to_string())?;
    let origin = notice.source.kind.as_str();
    let trace = notice.source.source.as_deref();
    let metadata = notice
        .source
        .metadata
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|error| error.to_string())?;
    let timestamp = now();
    let pending = held.as_ref().and_then(|(_, _, inbox)| inbox.as_deref());
    let (inbox, inserted) = if let Some(inbox) = pending {
        let causes = merged(tx, inbox, notice.causes)?;
        let changed = tx
            .execute(
                r#"
                    UPDATE strand_inbox
                    SET content = ?2, source_type = ?3, source_ref = ?4,
                        source_metadata = ?5, coalesce_revision = ?6,
                        coalesce_causes = ?7
                    WHERE id = ?1 AND strand_id = ?8 AND coalesce_key = ?9
                    "#,
                params![
                    inbox,
                    blob,
                    origin,
                    trace,
                    metadata,
                    revision,
                    causes,
                    notice.strand,
                    notice.key
                ],
            )
            .map_err(|error| error.to_string())?;
        if changed != 1 {
            return Err("inbox notice slot points to a missing pending item".to_string());
        }
        (inbox.to_string(), false)
    } else {
        capacity(tx, notice.strand)?;
        let inbox = tag("inbox");
        let causes = serde_json::to_string(&notice.causes).map_err(|error| error.to_string())?;
        tx.execute(
            r#"
                INSERT INTO strand_inbox (
                    id, strand_id, message_kind, content, source_type, source_ref,
                    source_metadata, coalesce_key, coalesce_revision,
                    coalesce_causes, created_at
                )
                VALUES (?1, ?2, 'santi_system', ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                "#,
            params![
                inbox,
                notice.strand,
                blob,
                origin,
                trace,
                metadata,
                notice.key,
                revision,
                causes,
                timestamp
            ],
        )
        .map_err(|error| error.to_string())?;
        Database::new(tx).accept(&inbox, notice.strand, &timestamp)?;
        (inbox, true)
    };
    tx.execute(
        r#"
            INSERT INTO inbox_slots (
                strand_id, slot_key, revision, digest, inbox_id, updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            ON CONFLICT(strand_id, slot_key) DO UPDATE SET
                revision = excluded.revision,
                digest = excluded.digest,
                inbox_id = excluded.inbox_id,
                updated_at = excluded.updated_at
            "#,
        params![
            notice.strand,
            notice.key,
            revision,
            notice.digest,
            inbox,
            timestamp
        ],
    )
    .map_err(|error| error.to_string())?;
    Ok(Offered {
        inbox: Some(inbox),
        inserted,
    })
}

fn merged(
    conn: &rusqlite::Connection,
    inbox: &str,
    incoming: Vec<String>,
) -> Result<String, String> {
    let raw = conn
        .query_row(
            "SELECT coalesce_causes FROM strand_inbox WHERE id = ?1",
            [inbox],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .flatten();
    let mut causes = raw
        .map(|raw| serde_json::from_str::<BTreeSet<String>>(&raw))
        .transpose()
        .map_err(|error| error.to_string())?
        .unwrap_or_default();
    causes.extend(incoming);
    serde_json::to_string(&causes).map_err(|error| error.to_string())
}

fn capacity(conn: &rusqlite::Connection, strand: &str) -> Result<(), String> {
    let pending = conn
        .query_row(
            "SELECT COUNT(*) FROM strand_inbox WHERE strand_id = ?1",
            [strand],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| error.to_string())?;
    if pending < GATE {
        return Ok(());
    }
    Err(format!(
        "strand inbox is full ({pending} pending, gate {GATE})"
    ))
}
