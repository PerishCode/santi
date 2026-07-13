use rusqlite::params;
use serde_json::json;

use super::{provider_incident_key, runtime_incident_key};
use crate::store::{
    SantiStore,
    db::{complete_turn_in_conn, turn_by_id},
    errors::resolve_in_conn,
    im::enqueue_turn_in,
};
use crate::{IM_LABEL_PREFIX, ImDeliveryMode, StrandMessage, Turn, timestamp_now};

impl SantiStore {
    pub fn complete_turn(
        &self,
        turn_id: &str,
        assistant_message_seq: Option<i64>,
        provider: &str,
        model: &str,
        provider_response_id: Option<String>,
    ) -> Result<Turn, String> {
        self.complete_inner(
            turn_id,
            assistant_message_seq,
            provider,
            model,
            provider_response_id,
            None,
        )
    }

    pub(crate) fn complete_turn_reply(
        &self,
        turn_id: &str,
        assistant_message: Option<&StrandMessage>,
        provider: &str,
        model: &str,
        provider_response_id: Option<String>,
    ) -> Result<Turn, String> {
        self.complete_inner(
            turn_id,
            assistant_message.map(|message| message.relation.strand_seq),
            provider,
            model,
            provider_response_id,
            assistant_message,
        )
    }

    fn complete_inner(
        &self,
        turn_id: &str,
        assistant_message_seq: Option<i64>,
        provider: &str,
        model: &str,
        provider_response_id: Option<String>,
        assistant_message: Option<&StrandMessage>,
    ) -> Result<Turn, String> {
        let mut conn = self.conn.lock().unwrap();
        let now = timestamp_now();
        let provider_state = provider_response_id.as_ref().map(|response_id| {
            json!({
                "provider": provider,
                "opaque": { "response_id": response_id },
                "schema_version": "santi-v1"
            })
        });
        let provider_state = provider_state
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| error.to_string())?;
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let (strand_id, external_label): (String, Option<String>) = tx
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
            .map_err(|error| error.to_string())?;
        tx.execute(
            r#"
            UPDATE turns
            SET status = 'completed',
                end_strand_seq = (
                  SELECT next_seq - 1 FROM strands WHERE id = turns.strand_id
                ),
                updated_at = ?2,
                finished_at = ?2
            WHERE id = ?1
            "#,
            params![turn_id, now],
        )
        .map_err(|error| error.to_string())?;
        tx.execute(
            r#"
            UPDATE strands
            SET last_seen_strand_seq = COALESCE(?2, last_seen_strand_seq),
                provider_state = ?3,
                updated_at = ?4
            WHERE id = (SELECT strand_id FROM turns WHERE id = ?1)
            "#,
            params![turn_id, assistant_message_seq, provider_state, now],
        )
        .map_err(|error| error.to_string())?;
        complete_turn_in_conn(&tx, turn_id, &now)?;
        resolve_in_conn(
            &tx,
            &provider_incident_key(&strand_id),
            "provider.turn_succeeded",
            json!({
                "turn_id": turn_id,
                "provider": provider,
                "model": model,
                "provider_response_id": provider_response_id,
            }),
        )?;
        resolve_in_conn(
            &tx,
            &runtime_incident_key(&strand_id),
            "runtime.turn_succeeded",
            json!({
                "turn_id": turn_id,
                "provider": provider,
                "model": model,
            }),
        )?;
        if let (Some(_), Some(message)) = (
            external_label
                .as_deref()
                .and_then(|label| label.strip_prefix(IM_LABEL_PREFIX)),
            assistant_message.filter(|message| !message.content_text.trim().is_empty()),
        ) {
            enqueue_turn_in(
                &tx,
                &strand_id,
                turn_id,
                Some(&message.message.id),
                &message.content_text,
                ImDeliveryMode::Automatic,
                &now,
            )?;
        }
        tx.commit().map_err(|error| error.to_string())?;
        turn_by_id(&conn, turn_id)?.ok_or_else(|| "completed turn missing".to_string())
    }
}
