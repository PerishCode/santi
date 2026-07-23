use rusqlite::params;
use serde_json::json;

use crate::catalog;
use crate::message;
use crate::store::{Store, db::Database};
use crate::{now, tag, turn::Turn};

pub struct Completion<'a> {
    pub turn: &'a str,
    pub sequence: Option<i64>,
    pub provider: &'a str,
    pub model: &'a str,
    pub response: Option<String>,
}

impl Store {
    pub fn complete(&self, completion: Completion<'_>) -> Result<Turn, String> {
        self.finish(completion, None).map(|(turn, _)| turn)
    }

    pub(crate) fn finish(
        &self,
        completion: Completion<'_>,
        message: Option<&message::Placed>,
    ) -> Result<(Turn, Option<crate::event::Event>), String> {
        let mut conn = self.conn.lock().unwrap();
        let now = now();
        let state = completion.response.as_ref().map(|response| {
            json!({
                "provider": completion.provider,
                "opaque": { "response_id": response },
                "schema_version": "santi-v1"
            })
        });
        let state = state
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| error.to_string())?;
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let (strand, label): (String, Option<String>) = tx
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
            params![completion.turn, completion.sequence, state, now],
        )
        .map_err(|error| error.to_string())?;
        Database::new(&tx).complete(completion.turn, &now)?;
        Database::new(&tx).resolve(
            &catalog::PROVIDER_TURN_FAILED.key("strand", &strand),
            "provider.turn_succeeded",
            json!({
                "turn": completion.turn,
                "provider": completion.provider,
                "model": completion.model,
                "response": completion.response,
            }),
        )?;
        Database::new(&tx).resolve(
            &catalog::RUNTIME_TURN_FAILED.key("strand", &strand),
            "runtime.turn_succeeded",
            json!({
                "turn": completion.turn,
                "provider": completion.provider,
                "model": completion.model,
            }),
        )?;
        Database::new(&tx).resolve(
            &catalog::EXECUTION_BUDGET_EXCEEDED.key("strand", &strand),
            "execution_budget.turn_succeeded",
            json!({
                "turn": completion.turn,
                "provider": completion.provider,
                "model": completion.model,
            }),
        )?;
        let turned = if let (Some(label), Some(reply)) = (
            label.as_deref(),
            message.filter(|message| !message.text.trim().is_empty()),
        ) {
            let event = crate::event::Event {
                id: tag("tev"),
                strand: strand.clone(),
                turn: completion.turn.to_string(),
                label: label.to_string(),
                text: reply.text.clone(),
                completed: now.clone(),
            };
            let payload = serde_json::to_string(&event).map_err(|error| error.to_string())?;
            Database::new(&tx).queue(crate::store::db::Queued {
                id: &event.id,
                turn: &event.turn,
                label: &event.label,
                payload: &payload,
                created: &now,
            })?;
            Some(event)
        } else {
            None
        };
        tx.commit().map_err(|error| error.to_string())?;
        let turn = Database::new(&conn)
            .turn(completion.turn)?
            .ok_or_else(|| "completed turn missing".to_string())?;
        Ok((turn, turned))
    }
}
