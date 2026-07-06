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

#[cfg(test)]
mod tests {
    use crate::{DEFAULT_SOUL_ID, SantiStore};

    fn store() -> (SantiStore, tempfile::TempDir) {
        let temp = tempfile::tempdir().expect("temp dir");
        let store = SantiStore::open(temp.path().join("db")).expect("open store");
        (store, temp)
    }

    #[test]
    fn participant_is_idempotent_on_id() {
        let (store, _temp) = store();
        store.ensure_im_participant("operator", "human").unwrap();
        // A second ensure must not error or duplicate — a persistent participant.
        store.ensure_im_participant("operator", "human").unwrap();
    }

    #[test]
    fn inbox_delivers_and_cursors_by_seq() {
        let (store, _temp) = store();
        store.ensure_im_participant("alice", "human").unwrap();
        store.ensure_im_participant("bob", "human").unwrap();

        let first = store.enqueue_im_inbox("alice", Some("ss_1"), "hi").unwrap();
        let second = store
            .enqueue_im_inbox("alice", Some("ss_1"), "again")
            .unwrap();
        // Global monotonic cursor.
        assert!(second.seq > first.seq);
        // A different participant's mail never leaks into alice's inbox.
        store.enqueue_im_inbox("bob", None, "for bob").unwrap();

        // since=0 → all of alice's, in seq order; bob's excluded.
        let all = store.poll_im_inbox("alice", 0).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].content, "hi");
        assert_eq!(all[1].content, "again");
        assert_eq!(all[0].from_ref.as_deref(), Some("ss_1"));

        // The caller's high-water seq IS the ack — past `first`, only `second`.
        let tail = store.poll_im_inbox("alice", first.seq).unwrap();
        assert_eq!(tail.len(), 1);
        assert_eq!(tail[0].seq, second.seq);

        // Fully caught up → empty (no re-delivery).
        assert!(store.poll_im_inbox("alice", second.seq).unwrap().is_empty());
    }

    #[test]
    fn participant_resolves_from_im_conversation_label() {
        let (store, _temp) = store();
        // The IM send builds an `im:<participant>` conversation strand.
        let strand = store
            .find_or_create_strand_by_label(DEFAULT_SOUL_ID, "im:alice")
            .unwrap();
        assert_eq!(
            store
                .im_participant_for_strand(&strand.id)
                .unwrap()
                .as_deref(),
            Some("alice")
        );

        // A non-IM strand (no `im:` label) has no IM participant.
        let plain = store.create_strand().unwrap();
        assert!(
            store
                .im_participant_for_strand(&plain.id)
                .unwrap()
                .is_none()
        );
    }
}
