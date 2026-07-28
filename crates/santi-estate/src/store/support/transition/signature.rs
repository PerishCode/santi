pub(super) const VERSION: i64 = 39;

pub(super) fn exact(objects: &[String]) -> bool {
    objects == expected() || objects == residue()
}

pub(super) fn expected() -> Vec<String> {
    OBJECTS
        .lines()
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn residue() -> Vec<String> {
    let mut objects = expected();
    objects.extend(
        RETIRED
            .lines()
            .filter(|line| !line.is_empty())
            .map(str::to_string),
    );
    objects.sort();
    objects
}

const RETIRED: &str = r#"index|idx_im_inbox_participant_seq|im_inbox
index|idx_im_inbox_turn|im_inbox
index|idx_r_soul_session_messages_seq|r_soul_session_messages
index|idx_r_soul_session_messages_target_lookup|r_soul_session_messages
table|im_inbox|im_inbox
table|im_participants|im_participants
table|r_soul_session_messages|r_soul_session_messages"#;

const OBJECTS: &str = r#"index|idx_compacts_strand|compacts
index|idx_downstream_ingest_receipt|downstream_ingest
index|idx_error_incidents_active_key|error_incidents
index|idx_error_incidents_scope_time|error_incidents
index|idx_error_transitions_pending|error_transitions
index|idx_inbox_receipts_strand_state|inbox_receipts
index|idx_job_capabilities_expiry|job_capabilities
index|idx_jobs_soul_time|jobs
index|idx_jobs_state_time|jobs
index|idx_message_events_message_id_created_at|message_events
index|idx_messages_actor_created_at|messages
index|idx_messages_state_created_at|messages
index|idx_r_strand_entries_seq|r_strand_entries
index|idx_r_strand_entries_target_lookup|r_strand_entries
index|idx_receipt_transitions_receipt_time|receipt_transitions
index|idx_strand_effects_state_updated_at|strand_effects
index|idx_strand_effects_strand_created_at|strand_effects
index|idx_strand_effects_turn_created_at|strand_effects
index|idx_strand_inbox_coalesce|strand_inbox
index|idx_strand_inbox_strand_created_at|strand_inbox
index|idx_strands_external_label|strands
index|idx_strands_lineage|strands
index|idx_strands_soul_id|strands
index|idx_thinking_spans_turn_id_created_at|thinking_spans
index|idx_tool_calls_turn_id_created_at|tool_calls
index|idx_tool_results_tool_call_id|tool_results
index|idx_trace_records_name_opened_at|trace_records
index|idx_turn_outbox_label_seq|turn_outbox
index|idx_turn_outbox_seq|turn_outbox
index|idx_turns_strand_created_at|turns
index|idx_turns_strand_status_created_at|turns
index|idx_webhook_deliveries_receipt|webhook_deliveries
table|compacts|compacts
table|downstream_ingest|downstream_ingest
table|downstreams|downstreams
table|error_incidents|error_incidents
table|error_transitions|error_transitions
table|inbox_receipts|inbox_receipts
table|inbox_slots|inbox_slots
table|job_capabilities|job_capabilities
table|jobs|jobs
table|message_events|message_events
table|messages|messages
table|provider_replay_material|provider_replay_material
table|r_strand_entries|r_strand_entries
table|receipt_transitions|receipt_transitions
table|souls|souls
table|strand_effects|strand_effects
table|strand_inbox|strand_inbox
table|strands|strands
table|thinking_spans|thinking_spans
table|tool_calls|tool_calls
table|tool_results|tool_results
table|trace_records|trace_records
table|turn_outbox|turn_outbox
table|turn_stops|turn_stops
table|turns|turns
table|webhook_deliveries|webhook_deliveries
table|webhooks|webhooks"#;
