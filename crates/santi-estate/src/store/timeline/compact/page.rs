use super::super::decode_target;
use super::{Store, plan, read};
use keel::adapt::db::Sqlite;
use keel::{Op, Rank, Row, Tx, form};
use santi_model::compact;

pub(super) async fn read(
    store: &Store,
    tag: &str,
    keyword: Option<&str>,
    page_index: i64,
    page_size: i64,
) -> Result<Option<compact::Page>, String> {
    let Some(compact) = store.compact(tag).await? else {
        return Ok(None);
    };
    let entries = store
        .core
        .batch(async |tx| entries(tx, &compact).await)
        .await
        .map_err(read::error)?;
    let needle = keyword
        .map(str::trim)
        .filter(|keyword| !keyword.is_empty())
        .map(str::to_lowercase);
    let entries = entries
        .into_iter()
        .filter(|entry| {
            needle
                .as_ref()
                .is_none_or(|needle| entry.text.to_lowercase().contains(needle))
        })
        .collect::<Vec<_>>();
    let total = entries.len() as i64;
    let skip = page_index.max(0).saturating_mul(page_size.max(0)) as usize;
    let take = page_size.max(0) as usize;
    Ok(Some(compact::Page {
        compact: compact.id,
        first: compact.first,
        last: compact.last,
        total,
        page_index,
        page_size,
        entries: entries.into_iter().skip(skip).take(take).collect(),
    }))
}

async fn entries(
    tx: &mut Tx<'_, Sqlite>,
    compact: &compact::Compact,
) -> Result<Vec<compact::Entry>, keel::adapt::Error> {
    let strand = need(tx, "Strand", &compact.strand).await?;
    let first = need(tx, "Message", &compact.first).await?;
    let last = need(tx, "Message", &compact.last).await?;
    let from = plan::sequence(tx, strand.key(), first.key()).await?;
    let to = plan::sequence(tx, strand.key(), last.key()).await?;
    let rows = tx
        .ask(
            &form("StrandEntry")
                .when("strand", Op::Eq, &strand.key().to_string())
                .when("sequence", Op::Ge, &from.to_string())
                .when("sequence", Op::Le, &to.to_string())
                .order("sequence", Rank::Asc),
        )
        .await?
        .rows()
        .to_vec();
    let mut entries = Vec::with_capacity(rows.len());
    for row in rows {
        let kind = text(&row, "target_type")?;
        let target = text(&row, "target")?;
        entries.push(compact::Entry {
            seq: row
                .int("sequence")
                .ok_or_else(|| keel::adapt::Error::Adapt("entry sequence missing".into()))?,
            kind: decode_target(kind).map_err(keel::adapt::Error::Adapt)?,
            target: target.to_string(),
            text: render(tx, kind, target).await?,
        });
    }
    Ok(entries)
}

async fn render(
    tx: &mut Tx<'_, Sqlite>,
    kind: &str,
    tag: &str,
) -> Result<String, keel::adapt::Error> {
    match kind {
        "message" => {
            let row = need(tx, "Message", tag).await?;
            let content = text(&row, "content")?;
            serde_json::from_str::<santi_model::message::Content>(content)
                .map(|content| content.rendered())
                .map_err(|error| keel::adapt::Error::Adapt(error.to_string()))
        }
        "tool_call" => {
            let row = need(tx, "ToolCall", tag).await?;
            let arguments = json(text(&row, "arguments")?)?;
            Ok(format!(
                "[tool_call {}] {}",
                text(&row, "tool")?,
                rendered(&arguments)
            ))
        }
        "tool_result" => result(tx, tag).await,
        "thinking" => {
            let row = need(tx, "ThinkingSpan", tag).await?;
            Ok(row
                .text("summary")
                .map(|summary| format!("[thinking] {summary}"))
                .unwrap_or_default())
        }
        _ => Ok(String::new()),
    }
}

async fn result(tx: &mut Tx<'_, Sqlite>, tag: &str) -> Result<String, keel::adapt::Error> {
    let result = need(tx, "ToolResult", tag).await?;
    let key = result.key().to_string();
    if let Some(output) = tx
        .one(&form("ToolOutput").when("result", Op::Eq, &key))
        .await?
    {
        return Ok(format!(
            "[tool_result] {}",
            rendered(&json(text(&output, "output")?)?)
        ));
    }
    if let Some(failure) = tx
        .one(&form("ToolFailure").when("result", Op::Eq, &key))
        .await?
    {
        return Ok(format!("[tool_result error] {}", text(&failure, "error")?));
    }
    Ok("[tool_result]".to_string())
}

async fn need(tx: &mut Tx<'_, Sqlite>, unit: &str, tag: &str) -> Result<Row, keel::adapt::Error> {
    tx.one(&form(unit).when("tag", Op::Eq, tag))
        .await?
        .ok_or_else(|| keel::adapt::Error::Missing(format!("{unit} {tag}")))
}

fn json(value: &str) -> Result<serde_json::Value, keel::adapt::Error> {
    serde_json::from_str(value).map_err(|error| keel::adapt::Error::Adapt(error.to_string()))
}

fn rendered(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn text<'a>(row: &'a Row, field: &str) -> Result<&'a str, keel::adapt::Error> {
    row.text(field)
        .ok_or_else(|| keel::adapt::Error::Adapt(format!("compact entry {field} missing")))
}
