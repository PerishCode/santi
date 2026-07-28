use santi_model::compact;
use serde_json::json;

pub(super) struct Range {
    pub(super) from: i64,
    pub(super) to: i64,
    pub(super) collapsed: i64,
}

pub(super) fn condensed(compact: &compact::Compact, fallback: Range) -> String {
    let metadata = compact.metadata.as_ref();
    let capsuled = metadata
        .and_then(|metadata| metadata.get("schema"))
        .and_then(|value| value.as_str())
        == Some("santi.compact_capsule.v1");
    let range = metadata.and_then(|metadata| metadata.get("range"));
    let from = number(range, "start_seq");
    let to = number(range, "end_seq");
    let collapsed = number(range, "collapsed_count");
    let before = nested(metadata, "before", "total");
    let after = nested(metadata, "after", "total");
    let budget = nested(metadata, "budget", "bytes");
    let ratio = metadata
        .and_then(|metadata| metadata.get("ratio"))
        .and_then(|value| value.as_f64());
    let header = json!({
        "schema": "santi.compact_projection.visible_header.v1",
        "compact": compact.id,
        "operation": noted(metadata, "operation")
            .unwrap_or(if capsuled { "manual_capsule" } else { "compact_projection" }),
        "declared_source": noted(metadata, "declared_source").unwrap_or("not_declared"),
        "source_trust": noted(metadata, "source_trust")
            .unwrap_or(if capsuled { "caller_declared" } else { "not_declared" }),
        "covered_message_range": {
            "first": compact.first,
            "last": compact.last,
        },
        "covered_strand_seq": {
            "start_seq": from.unwrap_or(fallback.from),
            "end_seq": to.unwrap_or(fallback.to),
        },
        "collapsed_entries": collapsed.unwrap_or(fallback.collapsed),
        "reason": noted(metadata, "reason").unwrap_or("not_declared"),
        "risk": noted(metadata, "risk").unwrap_or("not_declared"),
        "queryability": noted(metadata, "queryability")
            .unwrap_or("original spine remains queryable with compact query"),
        "originals_query": noted(metadata, "originals_query")
            .map(str::to_string)
            .unwrap_or_else(|| format!("santi compact query --compact-id {}", compact.id)),
        "context_estimate": {
            "pre_total_bytes": before,
            "post_total_bytes": after,
            "budget_input_bytes": budget,
            "ratio": ratio,
        },
    });
    let header = serde_json::to_string_pretty(&header).unwrap_or_else(|_| header.to_string());
    format!(
        "[compact projection]\n{header}\n[/compact projection]\n<compact_summary>\n{}\n</compact_summary>",
        compact.summary
    )
}

fn number(value: Option<&serde_json::Value>, key: &str) -> Option<i64> {
    value
        .and_then(|value| value.get(key))
        .and_then(|value| value.as_i64())
}

fn nested(value: Option<&serde_json::Value>, outer: &str, inner: &str) -> Option<i64> {
    value
        .and_then(|value| value.get(outer))
        .and_then(|value| value.get(inner))
        .and_then(|value| value.as_i64())
}

fn noted<'a>(metadata: Option<&'a serde_json::Value>, key: &str) -> Option<&'a str> {
    metadata
        .and_then(|metadata| metadata.get(key))
        .and_then(|value| value.as_str())
}
