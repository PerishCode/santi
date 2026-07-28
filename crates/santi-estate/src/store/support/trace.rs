use crate::store::{Store, read};
use keel::{Rank, form};
use santi_model::trace;

pub struct TraceDraft<'a> {
    pub tag: &'a str,
    pub boot: &'a str,
    pub span: i64,
    pub parent: Option<i64>,
    pub name: &'a str,
    pub tags: &'a [trace::Tag],
    pub opened: &'a str,
    pub closed: &'a str,
}

impl Store {
    pub async fn record_trace(&self, draft: TraceDraft<'_>) -> Result<(), String> {
        let tags = serde_json::to_string(draft.tags).map_err(|error| error.to_string())?;
        let span = draft.span.to_string();
        let parent = draft.parent.map(|parent| parent.to_string());
        let mut fields = vec![
            ("tag", draft.tag),
            ("boot", draft.boot),
            ("span", span.as_str()),
            ("name", draft.name),
            ("tags", tags.as_str()),
            ("opened", draft.opened),
            ("closed", draft.closed),
        ];
        if let Some(parent) = parent.as_deref() {
            fields.push(("parent", parent));
        }
        self.core
            .put("TraceRecord", &fields)
            .await
            .map_err(read::error)?;
        Ok(())
    }

    pub async fn traces(&self, key: &str, value: &str) -> Result<Vec<trace::Record>, String> {
        let rows = self
            .core
            .ask(
                &form("TraceRecord")
                    .order("opened", Rank::Asc)
                    .order("span", Rank::Asc)
                    .order("tag", Rank::Asc),
            )
            .await
            .map_err(read::error)?;
        rows.rows()
            .iter()
            .map(decode)
            .filter_map(|record| match record {
                Ok(record)
                    if record
                        .tags
                        .iter()
                        .any(|tag| tag.key == key && tag.value == value) =>
                {
                    Some(Ok(record))
                }
                Ok(_) => None,
                Err(error) => Some(Err(error)),
            })
            .collect()
    }
}

fn decode(row: &keel::Row) -> Result<trace::Record, String> {
    Ok(trace::Record {
        name: read::text(row, "name")?.to_string(),
        tags: serde_json::from_str(read::text(row, "tags")?).map_err(|error| error.to_string())?,
        opened: read::text(row, "opened")?.to_string(),
        closed: read::text(row, "closed")?.to_string(),
    })
}
