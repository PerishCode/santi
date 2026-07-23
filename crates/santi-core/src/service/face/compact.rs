use serde_json::json;

use super::tools::provider_tools;
use crate::context::budget::estimate_provider_parts;
use crate::{
    CompactCapsuleOptions, CompactExecRequest, CompactExecResponse, CompactQueryResponse,
    ContextBudget, ContextEstimate,
};

use super::Service;

impl Service {
    pub fn compact_exec(
        &self,
        strand: &str,
        request: CompactExecRequest,
    ) -> Result<CompactExecResponse, String> {
        let summary = request.summary.trim();
        if summary.is_empty() {
            return Err("compact summary must not be empty".to_string());
        }
        let strand = self
            .store
            .strand(strand)?
            .ok_or_else(|| "strand not found".to_string())?;
        let (from, to) = self.resolve_compact_boundaries(&strand.id, &request)?;
        let before = self.current_context_estimate(&strand.id)?;
        if request.dry {
            let mut response = self.store.preview_compact(&strand.id, &from, &to)?;
            response.before = Some(before);
            if let Some(capsule) = request.capsule.as_ref() {
                let metadata = compact_capsule_metadata(Capsule {
                    compact: Some(&response.compact),
                    capsule,
                    response: Some(&response),
                    before: response.before.as_ref(),
                    after: None,
                    budget: self.context_budget().as_ref(),
                    ratio: None,
                });
                let after =
                    self.estimate_preview_compact(&strand.id, &response, summary, metadata)?;
                let ratio = compact_compression_ratio(response.before.as_ref().unwrap(), &after);
                let metadata = compact_capsule_metadata(Capsule {
                    compact: Some(&response.compact),
                    capsule,
                    response: Some(&response),
                    before: response.before.as_ref(),
                    after: Some(&after),
                    budget: self.context_budget().as_ref(),
                    ratio,
                });
                let after =
                    self.estimate_preview_compact(&strand.id, &response, summary, metadata)?;
                response.ratio =
                    compact_compression_ratio(response.before.as_ref().unwrap(), &after);
                response.after = Some(after);
            }
            return Ok(response);
        }

        let initial_metadata = request.capsule.as_ref().map(|capsule| {
            compact_capsule_metadata(Capsule {
                compact: None,
                capsule,
                response: None,
                before: Some(&before),
                after: None,
                budget: self.context_budget().as_ref(),
                ratio: None,
            })
        });
        let mut response = self
            .store
            .create_compact_with_metadata(crate::store::Collapse {
                strand: &strand.id,
                from: &from,
                to: &to,
                summary,
                metadata: initial_metadata,
            })?;
        let mut after = self.current_context_estimate(&strand.id)?;
        let mut ratio = compact_compression_ratio(&before, &after);
        if let Some(capsule) = request.capsule.as_ref() {
            let metadata = compact_capsule_metadata(Capsule {
                compact: Some(&response.compact),
                capsule,
                response: Some(&response),
                before: Some(&before),
                after: Some(&after),
                budget: self.context_budget().as_ref(),
                ratio,
            });
            self.store
                .update_compact_metadata(&response.compact, metadata)?;
            after = self.current_context_estimate(&strand.id)?;
            ratio = compact_compression_ratio(&before, &after);
            let metadata = compact_capsule_metadata(Capsule {
                compact: Some(&response.compact),
                capsule,
                response: Some(&response),
                before: Some(&before),
                after: Some(&after),
                budget: self.context_budget().as_ref(),
                ratio,
            });
            self.store
                .update_compact_metadata(&response.compact, metadata)?;
        }
        response.active_incident_resolved =
            self.clear_context_incident(&strand.id, "compact_exec")?;
        if response.active_incident_resolved {
            self.poke_failed_receipts(&strand.id, "strand_send", None, "compact_recovery_poke");
        }
        response.before = Some(before);
        response.after = Some(after);
        response.ratio = ratio;
        Ok(response)
    }

    fn resolve_compact_boundaries(
        &self,
        strand: &str,
        request: &CompactExecRequest,
    ) -> Result<(String, String), String> {
        let from_id = request
            .first
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let to_id = request
            .last
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        match (from_id, to_id, request.from, request.to) {
            (Some(from), Some(to), None, None) => Ok((from.to_string(), to.to_string())),
            (None, None, Some(from), Some(to)) => {
                let from = self
                    .store
                    .message_id_at_seq(strand, from)?
                    .ok_or_else(|| format!("compact from {from} is not a message"))?;
                let to = self
                    .store
                    .message_id_at_seq(strand, to)?
                    .ok_or_else(|| format!("compact to {to} is not a message"))?;
                Ok((from, to))
            }
            _ => Err("compact requires either first/last or from/to".to_string()),
        }
    }

    fn estimate_preview_compact(
        &self,
        strand: &str,
        response: &CompactExecResponse,
        summary: &str,
        metadata: serde_json::Value,
    ) -> Result<ContextEstimate, String> {
        let input = self
            .store
            .assembly_input_preview(strand, response, summary, metadata)?;
        let instructions = self.system_prompt_text(strand)?;
        let tools = provider_tools();
        Ok(estimate_provider_parts(
            &input,
            Some(&instructions),
            Some(&tools),
        ))
    }

    pub fn compact_query(
        &self,
        compact: &str,
        keyword: Option<&str>,
        page_index: i64,
        page_size: i64,
    ) -> Result<Option<CompactQueryResponse>, String> {
        self.store
            .compact_query(compact, keyword, page_index, page_size)
    }
}

struct Capsule<'a> {
    compact: Option<&'a str>,
    capsule: &'a CompactCapsuleOptions,
    response: Option<&'a CompactExecResponse>,
    before: Option<&'a ContextEstimate>,
    after: Option<&'a ContextEstimate>,
    budget: Option<&'a ContextBudget>,
    ratio: Option<f64>,
}

const CAPSULE_SOURCE_BYTES: usize = 128;
const CAPSULE_REASON_BYTES: usize = 512;
const CAPSULE_RISK_BYTES: usize = 1024;
const CAPSULE_QUERYABILITY_BYTES: usize = 512;

fn compact_capsule_metadata(input: Capsule<'_>) -> serde_json::Value {
    let originals_query = input
        .compact
        .map(|id| format!("santi compact query --compact-id {id}"));
    let range = input.response.map(|response| {
        json!({
            "start_seq": response.start_seq,
            "end_seq": response.end_seq,
            "first": response.first,
            "last": response.last,
            "collapsed_count": response.collapsed_count,
            "absorbed": response.absorbed,
        })
    });
    json!({
        "schema": "santi.compact_capsule.v1",
        "operation": "manual_capsule",
        "compact": input.compact,
        "declared_source": cap_capsule_field(&input.capsule.source, CAPSULE_SOURCE_BYTES),
        "source_trust": "caller_declared",
        "reason": cap_capsule_field(&input.capsule.reason, CAPSULE_REASON_BYTES),
        "risk": cap_capsule_field(&input.capsule.risk, CAPSULE_RISK_BYTES),
        "queryability": input.capsule.queryability.as_ref().map(|value| {
            cap_capsule_field(value, CAPSULE_QUERYABILITY_BYTES)
        }),
        "originals_query": originals_query,
        "range": range,
        "before": input.before,
        "after": input.after,
        "budget": input.budget,
        "ratio": input.ratio,
    })
}

fn compact_compression_ratio(before: &ContextEstimate, after: &ContextEstimate) -> Option<f64> {
    if before.total <= 0 {
        return None;
    }
    Some(after.total as f64 / before.total as f64)
}

fn cap_capsule_field(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let suffix = " [truncated]";
    let suffix_bytes = suffix.len();
    let mut end = max_bytes.saturating_sub(suffix_bytes).min(value.len());
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{}", &value[..end], suffix)
}
