//! IM layer store methods — the plain messenger's own persistence, conceptually
//! ORTHOGONAL to the runtime (souls/strands/turns). See `schema.rs` (im_* tables)
//! and PHASE-08 CONVERGED MODEL v4. `source` addressing lives entirely here (the
//! IM's envelope), never in the runtime primitive or `strand_inbox`.

use rusqlite::{Connection, OptionalExtension, params};

use crate::{
    IM_LABEL_PREFIX, ImDelivery, ImDeliveryMode, ImInboxEntry, prefixed_id, timestamp_now,
};

use super::SantiStore;

impl SantiStore {
    /// Find-or-create a persistent IM participant, idempotent on the
    /// caller-declared stable `id`. `kind` is 'human' or 'soul'.
    pub fn ensure_im_participant(&self, id: &str, kind: &str) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
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

    /// Deliver a message into a (human/CLI) participant's passive inbox — the
    /// outbound crossing. Returns the stored entry with its cursor `seq`.
    /// `from_ref` is the soul strand that replied.
    pub fn enqueue_im_inbox(
        &self,
        participant_id: &str,
        from_ref: Option<&str>,
        content: &str,
    ) -> Result<ImInboxEntry, String> {
        let conn = self.conn.lock().unwrap();
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

    /// Deliver at most one participant reply for a concrete provider attempt.
    /// The turn id is the idempotency key shared by explicit early replies and
    /// the automatic turn-completion fallback.
    #[allow(clippy::too_many_arguments)]
    pub fn enqueue_turn_reply(
        &self,
        strand_id: &str,
        turn_id: &str,
        message_id: Option<&str>,
        content: &str,
        mode: ImDeliveryMode,
    ) -> Result<(ImInboxEntry, bool), String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let outcome = enqueue_turn_in(
            &tx,
            strand_id,
            turn_id,
            message_id,
            content,
            mode,
            &timestamp_now(),
        )?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(outcome)
    }

    /// Poll a participant's inbox for entries past the caller's cursor `since`
    /// (0 = from the beginning). Read-only, no ack — the caller's high-water
    /// `seq` IS the ack. Ordered by `seq` ascending.
    pub fn poll_im_inbox(
        &self,
        participant_id: &str,
        since: i64,
    ) -> Result<Vec<ImInboxEntry>, String> {
        let conn = self.conn.lock().unwrap();
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

    /// The reply-routing correlation: resolve a strand's IM participant from its
    /// `im:<id>` external label. A soul replying in an IM conversation strand
    /// reaches this participant. `None` if the strand is not an IM conversation.
    pub fn im_participant_for_strand(&self, strand_id: &str) -> Result<Option<String>, String> {
        let conn = self.conn.lock().unwrap();
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
}

#[allow(clippy::too_many_arguments)]
pub(super) fn enqueue_turn_in(
    conn: &Connection,
    strand_id: &str,
    turn_id: &str,
    message_id: Option<&str>,
    content: &str,
    mode: ImDeliveryMode,
    occurred_at: &str,
) -> Result<(ImInboxEntry, bool), String> {
    let (turn_strand, external_label): (String, Option<String>) = conn
        .query_row(
            r#"
            SELECT turn.strand_id, strand.external_label
            FROM turns AS turn
            JOIN strands AS strand ON strand.id = turn.strand_id
            WHERE turn.id = ?1
            "#,
            params![turn_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("unknown turn {turn_id}"))?;
    if turn_strand != strand_id {
        return Err(format!(
            "turn {turn_id} belongs to strand {turn_strand}, not {strand_id}"
        ));
    }
    let participant_id = external_label
        .as_deref()
        .and_then(|label| label.strip_prefix(IM_LABEL_PREFIX))
        .ok_or_else(|| format!("strand {strand_id} is not an IM conversation"))?;
    conn.execute(
        r#"
        INSERT INTO im_participants (id, kind, created_at)
        VALUES (?1, 'human', ?2)
        ON CONFLICT(id) DO NOTHING
        "#,
        params![participant_id, occurred_at],
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
                strand_id,
                turn_id,
                message_id,
                im_delivery_mode_db(&mode),
                content,
                occurred_at,
            ],
        )
        .map_err(|error| error.to_string())?
        == 1;
    let entry = reply_for_turn_in(conn, turn_id)?
        .ok_or_else(|| format!("IM reply for turn {turn_id} missing after insert"))?;
    if entry.participant_id != participant_id || entry.from_ref.as_deref() != Some(strand_id) {
        return Err(format!("IM reply idempotency collision for turn {turn_id}"));
    }
    Ok((entry, inserted))
}

pub(super) fn deliveries_for_receipt_in(
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
