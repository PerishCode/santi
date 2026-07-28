use santi_estate::Store;
use santi_model::{compact, message, strand};
use santi_provider::Item;
use serde_json::json;

mod overlay;
mod render;

pub async fn provider_input(store: &Store, strand: &str) -> Result<Vec<Item>, String> {
    assembled(store, strand, None).await
}

pub async fn provider_preview(
    store: &Store,
    strand: &str,
    report: &compact::Report,
    summary: &str,
    metadata: &serde_json::Value,
) -> Result<Vec<Item>, String> {
    let compact = compact::Compact {
        id: report.compact.clone(),
        strand: strand.to_string(),
        summary: summary.to_string(),
        first: report.first.clone(),
        last: report.last.clone(),
        created: None,
        metadata: Some(metadata.clone()),
    };
    let preview = overlay::Preview {
        compact,
        from: report.from,
        to: report.to,
        collapsed: report.collapsed,
        absorbed: &report.absorbed,
    };
    assembled(store, strand, Some(preview)).await
}

async fn assembled(
    store: &Store,
    strand: &str,
    preview: Option<overlay::Preview<'_>>,
) -> Result<Vec<Item>, String> {
    let entries = store.entries(strand).await?;
    let overlays = overlay::build(store, strand, &entries, preview).await?;
    let mut input = Vec::new();
    let mut cursor = 0usize;
    let mut overlaid = false;
    for entry in entries {
        while cursor < overlays.len() && overlays[cursor].to < entry.seq {
            cursor += 1;
            overlaid = false;
        }
        if cursor < overlays.len() && overlays[cursor].from <= entry.seq {
            if !overlaid {
                input.push(Item::Message {
                    role: "system".to_string(),
                    content: overlays[cursor].content.clone(),
                });
                overlaid = true;
            }
            continue;
        }
        if let Some(item) = projected(store, &entry).await? {
            input.push(item);
        }
    }
    Ok(input)
}

async fn projected(store: &Store, entry: &strand::Entry) -> Result<Option<Item>, String> {
    match entry.kind {
        strand::Target::Message => message(store, &entry.target).await,
        strand::Target::Thinking => thinking(store, &entry.target).await,
        strand::Target::ToolCall => call(store, &entry.target).await,
        strand::Target::ToolResult => reply(store, &entry.target).await,
        strand::Target::Compact => Err("compact cannot be a strand occurrence".to_string()),
    }
}

async fn message(store: &Store, target: &str) -> Result<Option<Item>, String> {
    let placed = store
        .message(target)
        .await?
        .ok_or_else(|| format!("message {target} missing"))?;
    let role = match (&placed.message.role, &placed.message.kind) {
        (message::Role::Soul, _) => "assistant",
        (message::Role::System, message::Kind::Text) => "user",
        (message::Role::System, message::Kind::SantiSystem) => "system",
    };
    let content = placed.message.content.rendered();
    Ok((!content.trim().is_empty()).then(|| Item::Message {
        role: role.to_string(),
        content,
    }))
}

async fn thinking(store: &Store, target: &str) -> Result<Option<Item>, String> {
    let span = store
        .thinking(target)
        .await?
        .ok_or_else(|| format!("thinking span {target} missing"))?;
    let Some(content) = span.summary.filter(|text| !text.trim().is_empty()) else {
        return Ok(None);
    };
    Ok(Some(Item::Reasoning {
        id: span.response,
        content,
    }))
}

async fn call(store: &Store, target: &str) -> Result<Option<Item>, String> {
    let call = store
        .call(target)
        .await?
        .ok_or_else(|| format!("tool call {target} missing"))?;
    let raw = serde_json::to_string(&call.arguments).map_err(|error| error.to_string())?;
    Ok(Some(Item::Call {
        call: call.id,
        name: call.tool,
        raw,
        item: None,
        mark: None,
    }))
}

async fn reply(store: &Store, target: &str) -> Result<Option<Item>, String> {
    let reply = store
        .reply(target)
        .await?
        .ok_or_else(|| format!("tool result {target} missing"))?;
    let output = serde_json::to_string(&json!({
        "ok": reply.error.is_none(),
        "output": reply.output,
        "error": reply.error,
    }))
    .map_err(|error| error.to_string())?;
    Ok(Some(Item::Output {
        call: reply.call,
        output,
    }))
}
