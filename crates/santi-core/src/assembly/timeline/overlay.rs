use super::render::{Range, condensed};
use santi_estate::Store;
use santi_model::{compact, strand};
use std::collections::HashMap;

pub(super) struct Preview<'a> {
    pub(super) compact: compact::Compact,
    pub(super) from: i64,
    pub(super) to: i64,
    pub(super) collapsed: i64,
    pub(super) absorbed: &'a [String],
}

pub(super) struct Overlay {
    pub(super) from: i64,
    pub(super) to: i64,
    pub(super) content: String,
}

pub(super) async fn build(
    store: &Store,
    strand: &str,
    entries: &[strand::Entry],
    preview: Option<Preview<'_>>,
) -> Result<Vec<Overlay>, String> {
    let seats = entries
        .iter()
        .filter(|entry| entry.kind == strand::Target::Message)
        .map(|entry| (entry.target.as_str(), entry.seq))
        .collect::<HashMap<_, _>>();
    let mut overlays = Vec::new();
    for compact in store.compacts(strand).await? {
        if preview
            .as_ref()
            .is_some_and(|preview| preview.absorbed.contains(&compact.id))
        {
            continue;
        }
        let from = seat(&seats, &compact.first)?;
        let to = seat(&seats, &compact.last)?;
        overlays.push(Overlay {
            from,
            to,
            content: condensed(
                &compact,
                Range {
                    from,
                    to,
                    collapsed: to.saturating_sub(from).saturating_add(1),
                },
            ),
        });
    }
    if let Some(preview) = preview {
        overlays.push(Overlay {
            from: preview.from,
            to: preview.to,
            content: condensed(
                &preview.compact,
                Range {
                    from: preview.from,
                    to: preview.to,
                    collapsed: preview.collapsed,
                },
            ),
        });
    }
    overlays.sort_by(|left, right| {
        left.from
            .cmp(&right.from)
            .then_with(|| left.to.cmp(&right.to))
    });
    Ok(overlays)
}

fn seat(seats: &HashMap<&str, i64>, message: &str) -> Result<i64, String> {
    seats
        .get(message)
        .copied()
        .ok_or_else(|| format!("compact boundary message {message} missing from strand"))
}
