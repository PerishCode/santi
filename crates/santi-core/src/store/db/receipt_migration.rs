use rusqlite::{Connection, params};

pub(super) fn migrate_v24_to_v25(conn: &Connection) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    tx.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS inbox_receipts (
            id TEXT PRIMARY KEY,
            strand_id TEXT NOT NULL,
            state TEXT NOT NULL CHECK (state IN (
                'accepted', 'mechanically_recovered', 'driving', 'turn_failed', 'completed'
            )),
            accepted_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_inbox_receipts_strand_state
        ON inbox_receipts(strand_id, state, accepted_at);

        CREATE TABLE IF NOT EXISTS receipt_transitions (
            id TEXT PRIMARY KEY,
            inbox_id TEXT NOT NULL,
            sequence INTEGER NOT NULL CHECK (sequence > 0),
            state TEXT NOT NULL CHECK (state IN (
                'accepted', 'mechanically_recovered', 'driving', 'turn_failed', 'completed'
            )),
            turn_id TEXT,
            incident_id TEXT,
            reconstructed_from TEXT,
            occurred_at TEXT NOT NULL,
            UNIQUE (inbox_id, sequence)
        );
        CREATE INDEX IF NOT EXISTS idx_receipt_transitions_receipt_time
        ON receipt_transitions(inbox_id, sequence);
        "#,
    )
    .map_err(|error| error.to_string())?;

    if table_exists(&tx, "message_events")? && table_exists(&tx, "turns")? {
        backfill_drained_receipts(&tx)?;
    }
    if table_exists(&tx, "strand_inbox")? {
        backfill_pending_receipts(&tx)?;
    }
    tx.commit().map_err(|error| error.to_string())
}

fn backfill_pending_receipts(conn: &Connection) -> Result<(), String> {
    let mut stmt = conn
        .prepare("SELECT id, strand_id, created_at FROM strand_inbox ORDER BY created_at, id")
        .map_err(|error| error.to_string())?;
    let pending = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    drop(stmt);
    for (inbox_id, strand_id, accepted_at) in pending {
        let exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM inbox_receipts WHERE id = ?1",
                params![&inbox_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if exists == 0 {
            super::receipts::insert_accepted_in_conn(conn, &inbox_id, &strand_id, &accepted_at)?;
            mark_reconstructed_from(conn, &inbox_id, &format!("v24:strand_inbox:{inbox_id}"))?;
        }
    }
    Ok(())
}

fn backfill_drained_receipts(conn: &Connection) -> Result<(), String> {
    struct DrainGroup {
        turn_id: String,
        strand_id: String,
        receipts: Vec<DrainReceipt>,
    }

    struct DrainReceipt {
        inbox_id: String,
        source_event_id: String,
    }

    let mut stmt = conn
        .prepare(
            "SELECT id, payload FROM message_events WHERE action = 'insert' ORDER BY created_at, id",
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| error.to_string())?;
    let mut groups: Vec<DrainGroup> = Vec::new();
    for row in rows {
        let (source_event_id, raw) = row.map_err(|error| error.to_string())?;
        let Ok(payload) = serde_json::from_str::<serde_json::Value>(&raw) else {
            continue;
        };
        if payload.get("kind").and_then(|value| value.as_str()) != Some("inbox_drain") {
            continue;
        }
        let Some(inbox_id) = payload.get("inbox_id").and_then(|value| value.as_str()) else {
            continue;
        };
        let Some(turn_id) = payload
            .get("committing_turn_id")
            .and_then(|value| value.as_str())
        else {
            continue;
        };
        let accepted_at = payload
            .get("enqueued_at")
            .and_then(|value| value.as_str())
            .or_else(|| payload.get("drained_at").and_then(|value| value.as_str()))
            .unwrap_or("1970-01-01T00:00:00Z");
        let strand_id: String = conn
            .query_row(
                "SELECT strand_id FROM turns WHERE id = ?1",
                params![turn_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        super::receipts::insert_accepted_in_conn(conn, inbox_id, &strand_id, accepted_at)?;
        if let Some(group) = groups.iter_mut().find(|group| group.turn_id == turn_id) {
            group.receipts.push(DrainReceipt {
                inbox_id: inbox_id.to_string(),
                source_event_id,
            });
        } else {
            groups.push(DrainGroup {
                turn_id: turn_id.to_string(),
                strand_id,
                receipts: vec![DrainReceipt {
                    inbox_id: inbox_id.to_string(),
                    source_event_id,
                }],
            });
        }
    }
    drop(stmt);

    for group in groups {
        let inbox_ids = group
            .receipts
            .iter()
            .map(|receipt| receipt.inbox_id.clone())
            .collect::<Vec<_>>();
        super::receipts::begin_turn_in_conn(
            conn,
            &group.strand_id,
            &group.turn_id,
            &inbox_ids,
            None,
        )?;
        let (status, finished_at): (String, Option<String>) = conn
            .query_row(
                "SELECT status, finished_at FROM turns WHERE id = ?1",
                params![&group.turn_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|error| error.to_string())?;
        if let Some(finished_at) = finished_at {
            match status.as_str() {
                "completed" => {
                    super::receipts::complete_turn_in_conn(conn, &group.turn_id, &finished_at)?
                }
                "failed" => {
                    super::receipts::fail_turn_in_conn(conn, &group.turn_id, None, &finished_at)?
                }
                _ => {}
            }
        }
        for receipt in group.receipts {
            mark_reconstructed_from(
                conn,
                &receipt.inbox_id,
                &format!("v24:message_event:{}", receipt.source_event_id),
            )?;
        }
    }
    Ok(())
}

fn mark_reconstructed_from(
    conn: &Connection,
    inbox_id: &str,
    source_ref: &str,
) -> Result<(), String> {
    conn.execute(
        "UPDATE receipt_transitions SET reconstructed_from = ?2 WHERE inbox_id = ?1",
        params![inbox_id, source_ref],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn table_exists(conn: &Connection, table: &str) -> Result<bool, String> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            params![table],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    Ok(count > 0)
}
