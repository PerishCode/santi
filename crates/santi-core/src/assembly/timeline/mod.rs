use santi_estate::Store;
use santi_model::{compact, message, strand};
use santi_provider::Item;
use serde_json::json;

mod overlay;
mod render;

pub struct Preview<'a> {
    pub report: &'a compact::Report,
    pub summary: &'a str,
    pub metadata: &'a serde_json::Value,
}

struct Timeline<'a> {
    store: &'a Store,
    strand: &'a str,
}

pub async fn provider_input(store: &Store, strand: &str) -> Result<Vec<Item>, String> {
    Timeline { store, strand }.assemble(None).await
}

pub async fn provider_preview(
    store: &Store,
    strand: &str,
    preview: Preview<'_>,
) -> Result<Vec<Item>, String> {
    let compact = compact::Compact {
        id: preview.report.compact.clone(),
        strand: strand.to_string(),
        summary: preview.summary.to_string(),
        first: preview.report.first.clone(),
        last: preview.report.last.clone(),
        created: None,
        metadata: Some(preview.metadata.clone()),
    };
    let preview = overlay::Preview {
        compact,
        from: preview.report.from,
        to: preview.report.to,
        collapsed: preview.report.collapsed,
        absorbed: &preview.report.absorbed,
    };
    Timeline { store, strand }.assemble(Some(preview)).await
}

impl Timeline<'_> {
    async fn assemble(&self, preview: Option<overlay::Preview<'_>>) -> Result<Vec<Item>, String> {
        let entries = self.store.entries(self.strand).await?;
        let overlays = overlay::build(self.store, self.strand, &entries, preview).await?;
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
            if let Some(item) = self.project(&entry).await? {
                input.push(item);
            }
        }
        Ok(input)
    }

    async fn project(&self, entry: &strand::Entry) -> Result<Option<Item>, String> {
        match entry.kind {
            strand::Target::Message => self.message(&entry.target).await,
            strand::Target::Thinking => self.thinking(&entry.target).await,
            strand::Target::ToolCall => self.call(&entry.target).await,
            strand::Target::ToolResult => self.reply(&entry.target).await,
            strand::Target::Compact => Err("compact cannot be a strand occurrence".to_string()),
        }
    }

    async fn message(&self, target: &str) -> Result<Option<Item>, String> {
        let placed = self
            .store
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

    async fn thinking(&self, target: &str) -> Result<Option<Item>, String> {
        let span = self
            .store
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

    async fn call(&self, target: &str) -> Result<Option<Item>, String> {
        let call = self
            .store
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

    async fn reply(&self, target: &str) -> Result<Option<Item>, String> {
        let reply = self
            .store
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
}
