use rusqlite::params;
use serde_json::Value;

use super::{
    Store,
    db::{Database, Prepared},
};
use crate::{effect, strand, thinking, tool};
use crate::{now, tag};

pub struct Invocation<'a> {
    pub turn: &'a str,
    pub call: &'a str,
    pub name: &'a str,
    pub arguments: &'a Value,
    pub provenance: &'a tool::Provenance,
}

impl Store {
    pub fn append_thinking_span(
        &self,
        turn: &str,
        response: Option<String>,
    ) -> Result<thinking::Span, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let thinking = tag("thinking");
        let now = now();
        let database = Database::new(&tx);
        let strand = database.holder(turn)?;
        tx.execute(
            r#"
            INSERT INTO thinking_spans (
              id, turn_id, provider_response_id, state, summary, completion_reason,
              error_text, created_at, updated_at, finished_at
            )
            VALUES (?1, ?2, ?3, 'running', NULL, NULL, NULL, ?4, ?4, NULL)
            "#,
            params![thinking, turn, response, now],
        )
        .map_err(|error| error.to_string())?;
        database.entered(&strand, strand::Target::Thinking, &thinking)?;
        tx.commit().map_err(|error| error.to_string())?;
        Database::new(&conn)
            .span(&thinking)?
            .ok_or_else(|| "created thinking_span missing".to_string())
    }

    pub fn attribute(
        &self,
        thinking_span_id: &str,
        response: Option<String>,
    ) -> Result<Option<thinking::Span>, String> {
        let conn = self.conn.lock().unwrap();
        let now = now();
        conn.execute(
            r#"
            UPDATE thinking_spans
            SET provider_response_id = COALESCE(?2, provider_response_id),
                updated_at = ?3
            WHERE id = ?1 AND state = 'running'
            "#,
            params![thinking_span_id, response, now],
        )
        .map_err(|error| error.to_string())?;
        Database::new(&conn).span(thinking_span_id)
    }

    pub fn summarize(
        &self,
        thinking_span_id: &str,
        summary: String,
    ) -> Result<Option<thinking::Span>, String> {
        let conn = self.conn.lock().unwrap();
        let now = now();
        conn.execute(
            r#"
            UPDATE thinking_spans
            SET summary = ?2,
                updated_at = ?3
            WHERE id = ?1 AND state <> 'failed'
            "#,
            params![thinking_span_id, summary, now],
        )
        .map_err(|error| error.to_string())?;
        Database::new(&conn).span(thinking_span_id)
    }

    pub fn complete_thinking_span(
        &self,
        thinking_span_id: &str,
        completion_reason: thinking::Reason,
    ) -> Result<Option<thinking::Span>, String> {
        self.finish_thinking_span(
            thinking_span_id,
            thinking::State::Completed,
            Some(completion_reason),
            None,
        )
    }

    pub fn fail_thinking_span(
        &self,
        thinking_span_id: &str,
        error: String,
    ) -> Result<Option<thinking::Span>, String> {
        self.finish_thinking_span(thinking_span_id, thinking::State::Failed, None, Some(error))
    }

    pub fn append_tool_call(&self, invocation: Invocation<'_>) -> Result<tool::Call, String> {
        self.append_effect_call(invocation, None)
            .map(|(call, _)| call)
    }

    pub fn append_effect_call(
        &self,
        invocation: Invocation<'_>,
        effect: Option<&str>,
    ) -> Result<(tool::Call, Option<effect::Effect>), String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let now = now();
        let database = Database::new(&tx);
        let strand = database.holder(invocation.turn)?;
        tx.execute(
            r#"
            INSERT INTO tool_calls (id, turn_id, tool_name, arguments, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
            params![
                invocation.call,
                invocation.turn,
                invocation.name,
                serde_json::to_string(invocation.arguments).map_err(|error| error.to_string())?,
                now
            ],
        )
        .map_err(|error| error.to_string())?;
        let texted = invocation
            .provenance
            .item
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| error.to_string())?;
        if texted.is_some()
            || invocation.provenance.mark.is_some()
            || invocation.provenance.response.is_some()
        {
            tx.execute(
                r#"
                INSERT INTO provider_replay_material
                    (tool_call_id, provider_family, kind, blob, item_id, response_id, schema_version, created_at)
                VALUES (?1, ?2, 'regenerable', ?3, ?4, ?5, ?6, ?7)
                "#,
                params![
                    invocation.call,
                    invocation.provenance.family,
                    texted,
                    invocation.provenance.mark,
                    invocation.provenance.response,
                    crate::VERSION,
                    now
                ],
            )
            .map_err(|error| error.to_string())?;
        }
        database.entered(&strand, strand::Target::ToolCall, invocation.call)?;
        let effect = effect
            .map(|kind| {
                Database::new(&tx).prepare(Prepared {
                    strand: &strand,
                    turn: invocation.turn,
                    call: invocation.call,
                    kind,
                    time: &now,
                })
            })
            .transpose()?;
        tx.commit().map_err(|error| error.to_string())?;
        let call = Database::new(&conn)
            .call(invocation.call)?
            .ok_or_else(|| "created tool_call missing".to_string())?;
        let effect = effect
            .as_deref()
            .map(|effect| Database::new(&conn).effect(effect))
            .transpose()?
            .flatten();
        Ok((call, effect))
    }

    pub fn append_tool_result(
        &self,
        call: &str,
        output: Option<Value>,
        error: Option<String>,
    ) -> Result<tool::Reply, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let reply = tag("tool_result");
        let now = now();
        let database = Database::new(&tx);
        let strand = database.caller(call)?;
        let output = output
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| error.to_string())?;
        tx.execute(
            r#"
            INSERT INTO tool_results (id, tool_call_id, output, error_text, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
            params![reply, call, output, error, now],
        )
        .map_err(|error| error.to_string())?;
        database.entered(&strand, strand::Target::ToolResult, &reply)?;
        tx.commit().map_err(|error| error.to_string())?;
        Database::new(&conn)
            .reply(&reply)?
            .ok_or_else(|| "created tool_result missing".to_string())
    }

    pub fn append_soul_assistant_text(
        &self,
        strand: &str,
        text: &str,
    ) -> Result<crate::message::Placed, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let soul: String = tx
            .query_row(
                "SELECT soul_id FROM strands WHERE id = ?1 LIMIT 1",
                params![strand],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        let message = tag("msg");
        let now = now();
        let blob = serde_json::to_string(&crate::message::Content::text(text))
            .map_err(|error| error.to_string())?;
        tx.execute(
            r#"
            INSERT INTO messages (
              id, actor_type, actor_id, message_kind, content, state, version, is_request,
              deleted_at, created_at, updated_at
            )
            VALUES (?1, 'soul', ?2, 'text', ?3, 'fixed', 1, 0, NULL, ?4, ?4)
            "#,
            params![message, soul, blob, now],
        )
        .map_err(|error| error.to_string())?;
        Database::new(&tx).entered(strand, strand::Target::Message, &message)?;
        tx.commit().map_err(|error| error.to_string())?;
        Database::new(&conn)
            .message(&message)?
            .ok_or_else(|| "created message missing".to_string())
    }

    fn finish_thinking_span(
        &self,
        thinking_span_id: &str,
        state: thinking::State,
        completion_reason: Option<thinking::Reason>,
        error: Option<String>,
    ) -> Result<Option<thinking::Span>, String> {
        let conn = self.conn.lock().unwrap();
        let now = now();
        conn.execute(
            r#"
            UPDATE thinking_spans
            SET state = ?2,
                completion_reason = ?3,
                error_text = ?4,
                updated_at = ?5,
                finished_at = ?5
            WHERE id = ?1 AND state = 'running'
            "#,
            params![
                thinking_span_id,
                state.encode(),
                completion_reason.as_ref().map(thinking::Reason::encode),
                error,
                now
            ],
        )
        .map_err(|error| error.to_string())?;
        Database::new(&conn).span(thinking_span_id)
    }
}
