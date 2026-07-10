use serde_json::json;

use crate::context_budget::estimate_provider_parts;
use crate::service_prompt::provider_tools;
use crate::{
    CompactCapsuleOptions, CompactExecRequest, CompactExecResponse, CompactQueryResponse,
    ContextBudget, ContextEstimate,
};

use super::SantiService;

impl SantiService {
    /// Compact a range of a strand's own timeline (self-involved: the soul
    /// runs this on itself). Creates the projection overlay directly over the
    /// addressed strand. The soul authors `summary`; the system only checks scale.
    pub fn compact_exec(
        &self,
        strand_id: &str,
        request: CompactExecRequest,
    ) -> Result<CompactExecResponse, String> {
        let summary = request.summary.trim();
        if summary.is_empty() {
            return Err("compact summary must not be empty".to_string());
        }
        let strand = self
            .store
            .strand(strand_id)?
            .ok_or_else(|| "strand not found".to_string())?;
        let (from, to) = self.resolve_compact_boundaries(&strand.id, &request)?;
        let pre_estimate = self.current_context_estimate(&strand.id)?;
        if request.dry_run {
            let mut response = self.store.preview_compact(&strand.id, &from, &to)?;
            response.pre_estimate = Some(pre_estimate);
            if let Some(capsule) = request.capsule.as_ref() {
                let metadata = compact_capsule_metadata(CompactCapsuleMetadataInput {
                    compact_id: Some(&response.compact_id),
                    capsule,
                    response: Some(&response),
                    pre_estimate: response.pre_estimate.as_ref(),
                    post_estimate: None,
                    budget: self.context_budget().as_ref(),
                    compression_ratio: None,
                });
                let post_estimate =
                    self.estimate_preview_compact(&strand.id, &response, summary, metadata)?;
                let compression_ratio = compact_compression_ratio(
                    response.pre_estimate.as_ref().unwrap(),
                    &post_estimate,
                );
                let metadata = compact_capsule_metadata(CompactCapsuleMetadataInput {
                    compact_id: Some(&response.compact_id),
                    capsule,
                    response: Some(&response),
                    pre_estimate: response.pre_estimate.as_ref(),
                    post_estimate: Some(&post_estimate),
                    budget: self.context_budget().as_ref(),
                    compression_ratio,
                });
                let post_estimate =
                    self.estimate_preview_compact(&strand.id, &response, summary, metadata)?;
                response.compression_ratio = compact_compression_ratio(
                    response.pre_estimate.as_ref().unwrap(),
                    &post_estimate,
                );
                response.post_estimate = Some(post_estimate);
            }
            return Ok(response);
        }

        let initial_metadata = request.capsule.as_ref().map(|capsule| {
            compact_capsule_metadata(CompactCapsuleMetadataInput {
                compact_id: None,
                capsule,
                response: None,
                pre_estimate: Some(&pre_estimate),
                post_estimate: None,
                budget: self.context_budget().as_ref(),
                compression_ratio: None,
            })
        });
        let mut response = self.store.create_compact_with_metadata(
            &strand.id,
            &from,
            &to,
            summary,
            initial_metadata,
        )?;
        let mut post_estimate = self.current_context_estimate(&strand.id)?;
        let mut compression_ratio = compact_compression_ratio(&pre_estimate, &post_estimate);
        if let Some(capsule) = request.capsule.as_ref() {
            let metadata = compact_capsule_metadata(CompactCapsuleMetadataInput {
                compact_id: Some(&response.compact_id),
                capsule,
                response: Some(&response),
                pre_estimate: Some(&pre_estimate),
                post_estimate: Some(&post_estimate),
                budget: self.context_budget().as_ref(),
                compression_ratio,
            });
            self.store
                .update_compact_metadata(&response.compact_id, metadata)?;
            post_estimate = self.current_context_estimate(&strand.id)?;
            compression_ratio = compact_compression_ratio(&pre_estimate, &post_estimate);
            let metadata = compact_capsule_metadata(CompactCapsuleMetadataInput {
                compact_id: Some(&response.compact_id),
                capsule,
                response: Some(&response),
                pre_estimate: Some(&pre_estimate),
                post_estimate: Some(&post_estimate),
                budget: self.context_budget().as_ref(),
                compression_ratio,
            });
            self.store
                .update_compact_metadata(&response.compact_id, metadata)?;
        }
        response.active_block_cleared = self.clear_context_block(&strand.id, "compact_exec")?;
        response.pre_estimate = Some(pre_estimate);
        response.post_estimate = Some(post_estimate);
        response.compression_ratio = compression_ratio;
        Ok(response)
    }

    fn resolve_compact_boundaries(
        &self,
        strand_id: &str,
        request: &CompactExecRequest,
    ) -> Result<(String, String), String> {
        let from_id = request
            .from_message_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let to_id = request
            .to_message_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        match (from_id, to_id, request.from_seq, request.to_seq) {
            (Some(from), Some(to), None, None) => Ok((from.to_string(), to.to_string())),
            (None, None, Some(from_seq), Some(to_seq)) => {
                let from = self
                    .store
                    .message_id_at_seq(strand_id, from_seq)?
                    .ok_or_else(|| format!("compact from_seq {from_seq} is not a message"))?;
                let to = self
                    .store
                    .message_id_at_seq(strand_id, to_seq)?
                    .ok_or_else(|| format!("compact to_seq {to_seq} is not a message"))?;
                Ok((from, to))
            }
            _ => Err(
                "compact requires either from_message_id/to_message_id or from_seq/to_seq"
                    .to_string(),
            ),
        }
    }

    fn estimate_preview_compact(
        &self,
        strand_id: &str,
        response: &CompactExecResponse,
        summary: &str,
        metadata: serde_json::Value,
    ) -> Result<ContextEstimate, String> {
        let input = self
            .store
            .assembly_input_preview(strand_id, response, summary, metadata)?;
        let instructions = self.system_prompt_text(strand_id)?;
        let tools = provider_tools();
        Ok(estimate_provider_parts(
            &input,
            Some(&instructions),
            Some(&tools),
        ))
    }

    pub fn compact_query(
        &self,
        compact_id: &str,
        keyword: Option<&str>,
        page_index: i64,
        page_size: i64,
    ) -> Result<Option<CompactQueryResponse>, String> {
        self.store
            .compact_query(compact_id, keyword, page_index, page_size)
    }
}

struct CompactCapsuleMetadataInput<'a> {
    compact_id: Option<&'a str>,
    capsule: &'a CompactCapsuleOptions,
    response: Option<&'a CompactExecResponse>,
    pre_estimate: Option<&'a ContextEstimate>,
    post_estimate: Option<&'a ContextEstimate>,
    budget: Option<&'a ContextBudget>,
    compression_ratio: Option<f64>,
}

const CAPSULE_SOURCE_BYTES: usize = 128;
const CAPSULE_REASON_BYTES: usize = 512;
const CAPSULE_RISK_BYTES: usize = 1024;
const CAPSULE_QUERYABILITY_BYTES: usize = 512;

fn compact_capsule_metadata(input: CompactCapsuleMetadataInput<'_>) -> serde_json::Value {
    let originals_query = input
        .compact_id
        .map(|id| format!("santi compact query --compact-id {id}"));
    let range = input.response.map(|response| {
        json!({
            "start_seq": response.start_seq,
            "end_seq": response.end_seq,
            "start_message_id": response.start_message_id,
            "end_message_id": response.end_message_id,
            "collapsed_count": response.collapsed_count,
            "absorbed": response.absorbed,
        })
    });
    json!({
        "schema": "santi.compact_capsule.v1",
        "operation": "manual_capsule",
        "compact_id": input.compact_id,
        "declared_source": cap_capsule_field(&input.capsule.source, CAPSULE_SOURCE_BYTES),
        "source_trust": "caller_declared",
        "reason": cap_capsule_field(&input.capsule.reason, CAPSULE_REASON_BYTES),
        "risk": cap_capsule_field(&input.capsule.risk, CAPSULE_RISK_BYTES),
        "queryability": input.capsule.queryability.as_ref().map(|value| {
            cap_capsule_field(value, CAPSULE_QUERYABILITY_BYTES)
        }),
        "originals_query": originals_query,
        "range": range,
        "pre_estimate": input.pre_estimate,
        "post_estimate": input.post_estimate,
        "budget": input.budget,
        "compression_ratio": input.compression_ratio,
    })
}

fn compact_compression_ratio(
    pre_estimate: &ContextEstimate,
    post_estimate: &ContextEstimate,
) -> Option<f64> {
    if pre_estimate.total_bytes <= 0 {
        return None;
    }
    Some(post_estimate.total_bytes as f64 / pre_estimate.total_bytes as f64)
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
