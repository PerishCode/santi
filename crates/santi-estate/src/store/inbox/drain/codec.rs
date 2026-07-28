use keel::Row;
use santi_model::{message, turn};

pub(super) struct Pending {
    pub(super) key: i64,
    pub(super) tag: String,
    pub(super) kind: String,
    pub(super) content: String,
    pub(super) source_type: Option<String>,
    pub(super) source_ref: Option<String>,
    pub(super) source_metadata: Option<String>,
    pub(super) coalesce_key: Option<String>,
    pub(super) causes: Option<String>,
    pub(super) created: String,
}

pub(super) fn decode(row: &Row) -> Result<Pending, keel::adapt::Error> {
    Ok(Pending {
        key: row.key(),
        tag: text(row, "tag")?,
        kind: text(row, "kind")?,
        content: text(row, "content")?,
        source_type: row.text("source_type").map(str::to_string),
        source_ref: row.text("source_ref").map(str::to_string),
        source_metadata: row.text("source_metadata").map(str::to_string),
        coalesce_key: row.text("coalesce_key").map(str::to_string),
        causes: row.text("coalesce_causes").map(str::to_string),
        created: text(row, "created")?,
    })
}

pub(super) fn aggregate(pending: &[Pending], now: &str) -> Result<String, keel::adapt::Error> {
    let mut lines = vec![
        "<system_message>".to_string(),
        "kind: inbox_attention".to_string(),
        "scope: strand_local".to_string(),
        "wake: true".to_string(),
        "obligation: false".to_string(),
        format!("assembled_at: {now}"),
        format!("items: {}", pending.len()),
    ];
    for item in pending {
        let content = serde_json::from_str::<message::Content>(&item.content)
            .map_err(|error| keel::adapt::Error::Adapt(error.to_string()))?;
        lines.push("---".to_string());
        if let Some(causes) = &item.causes {
            lines.push(format!("causes: {causes}"));
        }
        lines.push(content.rendered());
    }
    lines.push("</system_message>".to_string());
    serde_json::to_string(&message::Content::text(lines.join("\n")))
        .map_err(|error| keel::adapt::Error::Adapt(error.to_string()))
}

pub(super) fn trigger(trigger: &turn::Trigger) -> &'static str {
    match trigger {
        turn::Trigger::StrandSend => "strand_send",
        turn::Trigger::System => "system",
    }
}

fn text(row: &Row, field: &str) -> Result<String, keel::adapt::Error> {
    row.text(field)
        .map(str::to_string)
        .ok_or_else(|| keel::adapt::Error::Adapt(format!("inbox {field} missing")))
}
