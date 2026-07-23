use rusqlite::Row;
use santi_model::{compact::Compact, soul::Soul, strand::Strand, turn::Turn};
use serde_json::Value;

use super::*;
use santi_model::{downstream, effect, message, thinking, tool, turn, webhook};

impl Decode for Soul {
    fn decode(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            created: row.get(1)?,
            updated: row.get(2)?,
        })
    }
}

impl Decode for downstream::Credential {
    fn decode(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            prefix: row.get(1)?,
            digest: row.get(2)?,
            created: row.get(3)?,
            updated: row.get(4)?,
        })
    }
}

impl Decode for webhook::Subscription {
    fn decode(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            name: row.get(0)?,
            adaptor: row.get(1)?,
            soul: row.get(2)?,
            strategy: row.get(3)?,
            credential: row.get(4)?,
            created: row.get(5)?,
            updated: row.get(6)?,
        })
    }
}

impl Decode for Strand {
    fn decode(row: &Row<'_>) -> rusqlite::Result<Self> {
        let state: Option<String> = row.get(4)?;
        Ok(Self {
            id: row.get(0)?,
            soul: row.get(1)?,
            label: row.get(2)?,
            memory: row.get(3)?,
            state: state.and_then(|value| serde_json::from_str(&value).ok()),
            next: row.get(5)?,
            seen: row.get(6)?,
            parent: row.get(7)?,
            fork: row.get(8)?,
            created: row.get(9)?,
            updated: row.get(10)?,
        })
    }
}

impl Decode for message::Message {
    fn decode(row: &Row<'_>) -> rusqlite::Result<Self> {
        let blob: String = row.get(4)?;
        let content = serde_json::from_str::<message::Content>(&blob).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                4,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
        Ok(Self {
            id: row.get(0)?,
            role: message::Role::decode(row.get::<_, String>(1)?.as_str()),
            actor: row.get(2)?,
            kind: message::Kind::decode(row.get::<_, String>(3)?.as_str()),
            content,
            state: message::State::decode(row.get::<_, String>(5)?.as_str()),
            version: row.get(6)?,
            deleted: row.get(7)?,
            created: row.get(8)?,
            updated: row.get(9)?,
        })
    }
}

impl Decode for message::Event {
    fn decode(row: &Row<'_>) -> rusqlite::Result<Self> {
        let blob: String = row.get(6)?;
        let payload = serde_json::from_str::<Value>(&blob).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                6,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
        Ok(Self {
            id: row.get(0)?,
            message: row.get(1)?,
            action: row.get(2)?,
            role: message::Role::decode(row.get::<_, String>(3)?.as_str()),
            actor: row.get(4)?,
            base_version: row.get(5)?,
            payload,
            created: row.get(7)?,
        })
    }
}

impl Decode for message::Placed {
    fn decode(row: &Row<'_>) -> rusqlite::Result<Self> {
        let blob: String = row.get(8)?;
        let content = serde_json::from_str::<message::Content>(&blob).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                8,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
        let role = message::Role::decode(row.get::<_, String>(5)?.as_str());
        let kind = message::Kind::decode(row.get::<_, String>(7)?.as_str());
        let state = message::State::decode(row.get::<_, String>(9)?.as_str());
        let message = message::Message {
            id: row.get(4)?,
            role,
            actor: row.get(6)?,
            kind,
            content,
            state,
            version: row.get(10)?,
            deleted: row.get(11)?,
            created: row.get(12)?,
            updated: row.get(13)?,
        };
        let text = message.content.rendered();
        Ok(Self {
            relation: message::Relation {
                strand: row.get(0)?,
                message: row.get(1)?,
                seq: row.get(2)?,
                created: row.get(3)?,
            },
            message,
            text,
        })
    }
}

impl Decode for Turn {
    fn decode(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            strand: row.get(1)?,
            trigger: turn::Trigger::decode(row.get::<_, String>(2)?.as_str()),
            source: row.get(3)?,
            from: row.get(4)?,
            to: row.get(5)?,
            status: turn::Status::decode(row.get::<_, String>(6)?.as_str()),
            error: row.get(7)?,
            created: row.get(8)?,
            updated: row.get(9)?,
            finished: row.get(10)?,
        })
    }
}

impl Decode for tool::Call {
    fn decode(row: &Row<'_>) -> rusqlite::Result<Self> {
        let raw: String = row.get(3)?;
        let arguments = serde_json::from_str::<Value>(&raw).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                3,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
        Ok(Self {
            id: row.get(0)?,
            turn: row.get(1)?,
            tool: row.get(2)?,
            arguments,
            created: row.get(4)?,
        })
    }
}

impl Decode for tool::Reply {
    fn decode(row: &Row<'_>) -> rusqlite::Result<Self> {
        let output: Option<String> = row.get(2)?;
        Ok(Self {
            id: row.get(0)?,
            call: row.get(1)?,
            output: output.and_then(|value| serde_json::from_str(&value).ok()),
            error: row.get(3)?,
            created: row.get(4)?,
        })
    }
}

impl Decode for thinking::Span {
    fn decode(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            turn: row.get(1)?,
            response: row.get(2)?,
            state: thinking::State::decode(row.get::<_, String>(3)?.as_str()),
            summary: row.get(4)?,
            completion_reason: row
                .get::<_, Option<String>>(5)?
                .as_deref()
                .map(thinking::Reason::decode),
            error: row.get(6)?,
            created: row.get(7)?,
            updated: row.get(8)?,
            finished: row.get(9)?,
        })
    }
}

impl Decode for Compact {
    fn decode(row: &Row<'_>) -> rusqlite::Result<Self> {
        let blob: Option<String> = row.get(6)?;
        Ok(Self {
            id: row.get(0)?,
            strand: row.get(1)?,
            summary: row.get(2)?,
            first: row.get(3)?,
            last: row.get(4)?,
            created: row.get(5)?,
            metadata: blob.and_then(|value| serde_json::from_str(&value).ok()),
        })
    }
}

impl Decode for effect::Effect {
    fn decode(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            strand: row.get(1)?,
            turn: row.get(2)?,
            call: row.get(3)?,
            kind: row.get(4)?,
            state: effect::State::decode(&row.get::<_, String>(5)?),
            result: row.get(6)?,
            error: row.get(7)?,
            created: row.get(8)?,
            updated: row.get(9)?,
            dispatched: row.get(10)?,
            settled: row.get(11)?,
        })
    }
}
