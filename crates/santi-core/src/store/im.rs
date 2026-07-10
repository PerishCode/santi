//! IM layer store methods — the plain messenger's own persistence, conceptually
//! ORTHOGONAL to the runtime (souls/strands/turns). See `schema.rs` (im_* tables)
//! and PHASE-08 CONVERGED MODEL v4. `source` addressing lives entirely here (the
//! IM's envelope), never in the runtime primitive or `strand_inbox`.

use rusqlite::{OptionalExtension, params};

use crate::{IM_LABEL_PREFIX, ImInboxEntry, prefixed_id, timestamp_now};

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
            content: content.to_string(),
            created_at: now,
        })
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
                SELECT seq, id, participant_id, from_ref, content, created_at
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
                    content: row.get(4)?,
                    created_at: row.get(5)?,
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
