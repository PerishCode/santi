use rusqlite::params;
use serde_json::json;

use super::{provider_incident_key, runtime_incident_key};
use crate::store::{SantiStore, db::Database, execution_budget_incident_key};
use crate::{StrandMessage, Turn, prefixed_id, timestamp_now};

pub struct Completion<'a> {
    pub turn: &'a str,
    pub sequence: Option<i64>,
    pub provider: &'a str,
    pub model: &'a str,
    pub response: Option<String>,
}

impl SantiStore {
    pub fn complete_turn(&self, completion: Completion<'_>) -> Result<Turn, String> {
        self.complete(completion, None).map(|(turn, _)| turn)
    }

    pub(crate) fn complete(
        &self,
        completion: Completion<'_>,
        message: Option<&StrandMessage>,
    ) -> Result<(Turn, Option<crate::TurnEvent>), String> {
        let mut conn = self.conn.lock().unwrap();
        let now = timestamp_now();
        let provider_state = completion.response.as_ref().map(|response| {
            json!({
                "provider": completion.provider,
                "opaque": { "response_id": response },
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
                params![completion.turn],
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
            params![completion.turn, now],
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
            params![completion.turn, completion.sequence, provider_state, now],
        )
        .map_err(|error| error.to_string())?;
        Database::new(&tx).complete_turn(completion.turn, &now)?;
        Database::new(&tx).resolve_incident(
            &provider_incident_key(&strand_id),
            "provider.turn_succeeded",
            json!({
                "turn_id": completion.turn,
                "provider": completion.provider,
                "model": completion.model,
                "provider_response_id": completion.response,
            }),
        )?;
        Database::new(&tx).resolve_incident(
            &runtime_incident_key(&strand_id),
            "runtime.turn_succeeded",
            json!({
                "turn_id": completion.turn,
                "provider": completion.provider,
                "model": completion.model,
            }),
        )?;
        Database::new(&tx).resolve_incident(
            &execution_budget_incident_key(&strand_id),
            "execution_budget.turn_succeeded",
            json!({
                "turn_id": completion.turn,
                "provider": completion.provider,
                "model": completion.model,
            }),
        )?;
        let turn_event = if let (Some(label), Some(reply)) = (
            external_label.as_deref(),
            message.filter(|message| !message.content_text.trim().is_empty()),
        ) {
            let event = crate::TurnEvent {
                id: prefixed_id("tev"),
                strand_id: strand_id.clone(),
                turn_id: completion.turn.to_string(),
                external_label: label.to_string(),
                final_text: reply.content_text.clone(),
                completed_at: now.clone(),
            };
            let payload = serde_json::to_string(&event).map_err(|error| error.to_string())?;
            Database::new(&tx).insert_turn_outbox(crate::store::db::TurnOutboxInsert {
                id: &event.id,
                turn: &event.turn_id,
                label: &event.external_label,
                payload: &payload,
                created: &now,
            })?;
            Some(event)
        } else {
            None
        };
        tx.commit().map_err(|error| error.to_string())?;
        let turn = Database::new(&conn)
            .turn_by_id(completion.turn)?
            .ok_or_else(|| "completed turn missing".to_string())?;
        Ok((turn, turn_event))
    }
}
