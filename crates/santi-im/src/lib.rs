use rusqlite::{Connection, OptionalExtension, params};

use santi_model::{
    IM_LABEL_PREFIX, ImDelivery, ImDeliveryMode, ImInboxEntry, prefixed_id, timestamp_now,
};

pub struct Reply<'a> {
    pub strand: &'a str,
    pub turn: &'a str,
    pub message: Option<&'a str>,
    pub content: &'a str,
    pub mode: ImDeliveryMode,
}

pub fn ensure_participant(conn: &Connection, id: &str, kind: &str) -> Result<(), String> {
    let now = timestamp_now();
    conn.execute(
        r#"
        INSERT INTO im_participants (id, kind, created_at)
        VALUES (?1, ?2, ?3)
        ON CONFLICT(id) DO NOTHING
        "#,
        params![id, kind, now],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

pub fn enqueue_inbox(
    conn: &Connection,
    participant_id: &str,
    from_ref: Option<&str>,
    content: &str,
) -> Result<ImInboxEntry, String> {
    let id = prefixed_id("imx");
    let now = timestamp_now();
    conn.execute(
        r#"
        INSERT INTO im_inbox (id, participant_id, from_ref, content, created_at)
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
        params![id, participant_id, from_ref, content, now],
    )
    .map_err(|error| error.to_string())?;
    Ok(ImInboxEntry {
        seq: conn.last_insert_rowid(),
        id,
        participant_id: participant_id.to_string(),
        from_ref: from_ref.map(str::to_string),
        turn_id: None,
        message_id: None,
        delivery_mode: None,
        content: content.to_string(),
        created_at: now,
    })
}

pub fn poll_inbox(
    conn: &Connection,
    participant_id: &str,
    since: i64,
) -> Result<Vec<ImInboxEntry>, String> {
    let mut stmt = conn
        .prepare(
            r#"
            SELECT seq, id, participant_id, from_ref, turn_id, message_id,
                   delivery_mode, content, created_at
            FROM im_inbox
            WHERE participant_id = ?1 AND seq > ?2
            ORDER BY seq ASC
            "#,
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(params![participant_id, since], |row| {
            Ok(ImInboxEntry {
                seq: row.get(0)?,
                id: row.get(1)?,
                participant_id: row.get(2)?,
                from_ref: row.get(3)?,
                turn_id: row.get(4)?,
                message_id: row.get(5)?,
                delivery_mode: row
                    .get::<_, Option<String>>(6)?
                    .map(|mode| delivery_mode_from_db(&mode)),
                content: row.get(7)?,
                created_at: row.get(8)?,
            })
        })
        .map_err(|error| error.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|error| error.to_string())?);
    }
    Ok(out)
}

pub fn participant_for_strand(
    conn: &Connection,
    strand_id: &str,
) -> Result<Option<String>, String> {
    let label: Option<Option<String>> = conn
        .query_row(
            "SELECT external_label FROM strands WHERE id = ?1",
            params![strand_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    Ok(label
        .flatten()
        .and_then(|label| label.strip_prefix(IM_LABEL_PREFIX).map(str::to_string)))
}

pub fn enqueue_turn_in(
    conn: &Connection,
    reply: Reply<'_>,
    at: &str,
) -> Result<(ImInboxEntry, bool), String> {
    let Reply {
        strand,
        turn,
        message,
        content,
        mode,
    } = reply;
    let (turn_strand, external_label): (String, Option<String>) = conn
        .query_row(
            r#"
            SELECT turn.strand_id, strand.external_label
            FROM turns AS turn
            JOIN strands AS strand ON strand.id = turn.strand_id
            WHERE turn.id = ?1
            "#,
            params![turn],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("unknown turn {turn}"))?;
    if turn_strand != strand {
        return Err(format!(
            "turn {turn} belongs to strand {turn_strand}, not {strand}"
        ));
    }
    let participant_id = external_label
        .as_deref()
        .and_then(|label| label.strip_prefix(IM_LABEL_PREFIX))
        .ok_or_else(|| format!("strand {strand} is not an IM conversation"))?;
    conn.execute(
        r#"
        INSERT INTO im_participants (id, kind, created_at)
        VALUES (?1, 'human', ?2)
        ON CONFLICT(id) DO NOTHING
        "#,
        params![participant_id, at],
    )
    .map_err(|error| error.to_string())?;
    let id = prefixed_id("imx");
    let inserted = conn
        .execute(
            r#"
            INSERT OR IGNORE INTO im_inbox (
              id, participant_id, from_ref, turn_id, message_id,
              delivery_mode, content, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            params![
                id,
                participant_id,
                strand,
                turn,
                message,
                im_delivery_mode_db(&mode),
                content,
                at,
            ],
        )
        .map_err(|error| error.to_string())?
        == 1;
    let entry = reply_for_turn_in(conn, turn)?
        .ok_or_else(|| format!("IM reply for turn {turn} missing after insert"))?;
    if entry.participant_id != participant_id || entry.from_ref.as_deref() != Some(strand) {
        return Err(format!("IM reply idempotency collision for turn {turn}"));
    }
    Ok((entry, inserted))
}

pub fn deliveries_for_receipt_in(
    conn: &Connection,
    inbox_id: &str,
) -> Result<Vec<ImDelivery>, String> {
    let mut stmt = conn
        .prepare(
            r#"
            SELECT DISTINCT delivery.seq, delivery.id, delivery.participant_id,
                   delivery.from_ref, delivery.turn_id, delivery.message_id,
                   delivery.delivery_mode, delivery.created_at
            FROM im_inbox AS delivery
            JOIN receipt_transitions AS receipt ON receipt.turn_id = delivery.turn_id
            WHERE receipt.inbox_id = ?1
            ORDER BY delivery.seq
            "#,
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(params![inbox_id], |row| {
            let mode = row.get::<_, String>(6)?;
            Ok(ImDelivery {
                seq: row.get(0)?,
                id: row.get(1)?,
                participant_id: row.get(2)?,
                strand_id: row.get(3)?,
                turn_id: row.get(4)?,
                message_id: row.get(5)?,
                delivery_mode: delivery_mode_from_db(&mode),
                created_at: row.get(7)?,
            })
        })
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

fn reply_for_turn_in(conn: &Connection, turn_id: &str) -> Result<Option<ImInboxEntry>, String> {
    conn.query_row(
        r#"
        SELECT seq, id, participant_id, from_ref, turn_id, message_id,
               delivery_mode, content, created_at
        FROM im_inbox WHERE turn_id = ?1
        "#,
        params![turn_id],
        |row| {
            Ok(ImInboxEntry {
                seq: row.get(0)?,
                id: row.get(1)?,
                participant_id: row.get(2)?,
                from_ref: row.get(3)?,
                turn_id: row.get(4)?,
                message_id: row.get(5)?,
                delivery_mode: row
                    .get::<_, Option<String>>(6)?
                    .map(|mode| delivery_mode_from_db(&mode)),
                content: row.get(7)?,
                created_at: row.get(8)?,
            })
        },
    )
    .optional()
    .map_err(|error| error.to_string())
}

fn im_delivery_mode_db(mode: &ImDeliveryMode) -> &'static str {
    match mode {
        ImDeliveryMode::Explicit => "explicit",
        ImDeliveryMode::Automatic => "automatic",
    }
}

fn delivery_mode_from_db(mode: &str) -> ImDeliveryMode {
    match mode {
        "explicit" => ImDeliveryMode::Explicit,
        _ => ImDeliveryMode::Automatic,
    }
}

pub fn deliver_reply(conn: &Connection, event: &santi_protocol::ReplyEvent) -> Result<(), String> {
    conn.execute(
        r#"
        INSERT INTO im_participants (id, kind, created_at)
        VALUES (?1, 'human', ?2)
        ON CONFLICT(id) DO NOTHING
        "#,
        params![event.participant_id, timestamp_now()],
    )
    .map_err(|error| error.to_string())?;
    conn.execute(
        r#"
        INSERT OR IGNORE INTO im_inbox (
          id, participant_id, from_ref, turn_id, message_id,
          delivery_mode, content, created_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
        params![
            prefixed_id("imx"),
            event.participant_id,
            event.strand_id,
            event.turn_id,
            event.message_id,
            im_delivery_mode_db(&event.mode),
            event.content,
            timestamp_now(),
        ],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}
