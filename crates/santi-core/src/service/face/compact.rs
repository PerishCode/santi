use serde_json::json;

use super::tools::tools;
use crate::context::budget::estimated;

use super::Service;
use crate::{budget, compact};

impl Service {
    pub async fn exec(
        &self,
        strand: &str,
        request: compact::Exec,
    ) -> Result<compact::Report, String> {
        let summary = request.summary.trim();
        if summary.is_empty() {
            return Err("compact summary must not be empty".to_string());
        }
        let strand = self
            .store
            .strand(strand)
            .await?
            .ok_or_else(|| "strand not found".to_string())?;
        let (from, to) = self.bounded2(&strand.id, &request).await?;
        let before = self.estimate(&strand.id).await?;
        if request.dry {
            let mut response = self
                .store
                .preview_compact(&crate::tag("compact"), &strand.id, &from, &to)
                .await?;
            response.before = Some(before);
            if let Some(capsule) = request.capsule.as_ref() {
                let metadata = encapsulated(Capsule {
                    compact: Some(&response.compact),
                    capsule,
                    response: Some(&response),
                    before: response.before.as_ref(),
                    after: None,
                    budget: self.budget().as_ref(),
                    ratio: None,
                });
                let after = self
                    .foreseen(&strand.id, &response, summary, metadata)
                    .await?;
                let ratio = squeezed(response.before.as_ref().unwrap(), &after);
                let metadata = encapsulated(Capsule {
                    compact: Some(&response.compact),
                    capsule,
                    response: Some(&response),
                    before: response.before.as_ref(),
                    after: Some(&after),
                    budget: self.budget().as_ref(),
                    ratio,
                });
                let after = self
                    .foreseen(&strand.id, &response, summary, metadata)
                    .await?;
                response.ratio = squeezed(response.before.as_ref().unwrap(), &after);
                response.after = Some(after);
            }
            return Ok(response);
        }

        let initial = request.capsule.as_ref().map(|capsule| {
            encapsulated(Capsule {
                compact: None,
                capsule,
                response: None,
                before: Some(&before),
                after: None,
                budget: self.budget().as_ref(),
                ratio: None,
            })
        });
        let compact_tag = crate::tag("compact");
        let mut response = self
            .store
            .create_compact(santi_estate::CompactDraft {
                tag: &compact_tag,
                strand: &strand.id,
                first: &from,
                last: &to,
                summary,
                metadata: initial.as_ref(),
                created: &crate::now(),
            })
            .await?;
        let mut after = self.estimate(&strand.id).await?;
        let mut ratio = squeezed(&before, &after);
        if let Some(capsule) = request.capsule.as_ref() {
            let metadata = encapsulated(Capsule {
                compact: Some(&response.compact),
                capsule,
                response: Some(&response),
                before: Some(&before),
                after: Some(&after),
                budget: self.budget().as_ref(),
                ratio,
            });
            self.store
                .annotate_compact(&response.compact, &metadata)
                .await?;
            after = self.estimate(&strand.id).await?;
            ratio = squeezed(&before, &after);
            let metadata = encapsulated(Capsule {
                compact: Some(&response.compact),
                capsule,
                response: Some(&response),
                before: Some(&before),
                after: Some(&after),
                budget: self.budget().as_ref(),
                ratio,
            });
            self.store
                .annotate_compact(&response.compact, &metadata)
                .await?;
        }
        response.active_incident_resolved = self.absolve(&strand.id, "compact_exec").await?;
        if response.active_incident_resolved {
            self.poked(&strand.id, "strand_send", None, "compact_recovery_poke")
                .await;
        }
        response.before = Some(before);
        response.after = Some(after);
        response.ratio = ratio;
        Ok(response)
    }

    async fn bounded2(
        &self,
        strand: &str,
        request: &compact::Exec,
    ) -> Result<(String, String), String> {
        let from = request
            .first
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let to = request
            .last
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        match (from, to, request.from, request.to) {
            (Some(from), Some(to), None, None) => Ok((from.to_string(), to.to_string())),
            (None, None, Some(from), Some(to)) => {
                let from = self
                    .store
                    .seated(strand, from)
                    .await?
                    .ok_or_else(|| format!("compact from {from} is not a message"))?;
                let to = self
                    .store
                    .seated(strand, to)
                    .await?
                    .ok_or_else(|| format!("compact to {to} is not a message"))?;
                Ok((from, to))
            }
            _ => Err("compact requires either first/last or from/to".to_string()),
        }
    }

    async fn foreseen(
        &self,
        strand: &str,
        response: &compact::Report,
        summary: &str,
        metadata: serde_json::Value,
    ) -> Result<budget::Estimate, String> {
        let input =
            crate::provider_preview(&self.store, strand, response, summary, &metadata).await?;
        let instructions = self.wording(strand).await?;
        let tools = tools();
        Ok(estimated(&input, Some(&instructions), Some(&tools)))
    }

    pub async fn page(
        &self,
        compact: &str,
        keyword: Option<&str>,
        page_index: i64,
        page_size: i64,
    ) -> Result<Option<compact::Page>, String> {
        self.store
            .compact_page(compact, keyword, page_index, page_size)
            .await
    }
}

struct Capsule<'a> {
    compact: Option<&'a str>,
    capsule: &'a compact::Capsule,
    response: Option<&'a compact::Report>,
    before: Option<&'a budget::Estimate>,
    after: Option<&'a budget::Estimate>,
    budget: Option<&'a budget::Cap>,
    ratio: Option<f64>,
}

const SOURCING: usize = 128;
const REASONING: usize = 512;
const RISK: usize = 1024;
const QUERYABILITY: usize = 512;

fn encapsulated(input: Capsule<'_>) -> serde_json::Value {
    let originals = input
        .compact
        .map(|id| format!("santi compact query --compact-id {id}"));
    let range = input.response.map(|response| {
        json!({
            "start_seq": response.from,
            "end_seq": response.to,
            "first": response.first,
            "last": response.last,
            "collapsed_count": response.collapsed,
            "absorbed": response.absorbed,
        })
    });
    json!({
        "schema": "santi.compact_capsule.v1",
        "operation": "manual_capsule",
        "compact": input.compact,
        "declared_source": capped(&input.capsule.source, SOURCING),
        "source_trust": "caller_declared",
        "reason": capped(&input.capsule.reason, REASONING),
        "risk": capped(&input.capsule.risk, RISK),
        "queryability": input.capsule.queryability.as_ref().map(|value| {
            capped(value, QUERYABILITY)
        }),
        "originals_query": originals,
        "range": range,
        "before": input.before,
        "after": input.after,
        "budget": input.budget,
        "ratio": input.ratio,
    })
}

fn squeezed(before: &budget::Estimate, after: &budget::Estimate) -> Option<f64> {
    if before.total <= 0 {
        return None;
    }
    Some(after.total as f64 / before.total as f64)
}

fn capped(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let suffix = " [truncated]";
    let suffix = suffix.len();
    let mut end = max_bytes.saturating_sub(suffix).min(value.len());
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{}", &value[..end], suffix)
}
