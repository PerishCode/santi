use std::io::Write as _;
use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::StreamExt;

use crate::cli::{
    ClientDefaults, Command, CompactCommand, EffectCommand, StrandCommand, WatchFormat,
    split_send_args,
};
use crate::text::source::read_summary_file;
use crate::watch::{next_sse_frame, render_watch_event};

mod send;

pub use send::{Request, send};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

struct Capsule<'a> {
    from: Option<String>,
    to: Option<String>,
    start: Option<i64>,
    end: Option<i64>,
    summary: Option<String>,
    file: Option<String>,
    source: String,
    reason: String,
    risk: String,
    queryability: String,
    preview: bool,
    soul: Option<&'a str>,
}

pub(crate) async fn run_client(
    base_url: &str,
    bearer: Option<&str>,
    defaults: &ClientDefaults,
    command: Command,
) -> Result<()> {
    let client = build_client(bearer)?;
    let http = Http { client: &client };
    let base = base_url.trim_end_matches('/').to_string();
    match command {
        Command::Service { .. } => unreachable!("service is handled before the client path"),
        Command::Doctor { .. } => unreachable!("doctor is handled before the client path"),
        Command::Inbox(_) => unreachable!("inbox is handled before the client path"),
        Command::Upgrade { .. } => unreachable!("upgrade is handled before the client path"),
        Command::Health => http.get(&format!("{base}/api/v1/health")).await,
        Command::Errors {
            scope_kind,
            scope_id,
            limit,
        } => {
            http.get(&format!(
                "{base}/api/v1/errors/{scope_kind}/{scope_id}?limit={limit}"
            ))
            .await
        }
        Command::Receipt { inbox_id } => {
            http.get(&format!("{base}/api/v1/receipts/{inbox_id}"))
                .await
        }
        Command::Effect(EffectCommand::Query { effect_id }) => {
            http.get(&format!("{base}/api/v1/effects/{effect_id}"))
                .await
        }
        Command::Effect(EffectCommand::Resolve {
            effect_id,
            outcome,
            evidence,
        }) => {
            http.post(
                &format!("{base}/api/v1/effects/{effect_id}/resolve"),
                Some(serde_json::json!({
                    "outcome": outcome.as_api_str(),
                    "evidence": evidence,
                })),
            )
            .await
        }
        Command::Strand(StrandCommand::Create) => {
            http.post(&format!("{base}/api/v1/strands"), None).await
        }
        Command::Strand(StrandCommand::List) => http.get(&format!("{base}/api/v1/strands")).await,
        Command::Strand(StrandCommand::Get { id }) => {
            let id = defaults.resolve_strand(id)?;
            http.get(&format!("{base}/api/v1/strands/{id}")).await
        }
        Command::Strand(StrandCommand::Messages { id }) => {
            let id = defaults.resolve_strand(id)?;
            http.get(&format!("{base}/api/v1/strands/{id}/messages"))
                .await
        }
        Command::Strand(StrandCommand::Runtime { id }) => {
            let id = defaults.resolve_strand(id)?;
            http.get(&format!("{base}/api/v1/strands/{id}/runtime"))
                .await
        }
        Command::Strand(StrandCommand::Budget { id }) => {
            let id = defaults.resolve_strand(id)?;
            http.get(&format!("{base}/api/v1/strands/{id}/budget"))
                .await
        }
        Command::Strand(StrandCommand::Errors { id, limit }) => {
            let id = defaults.resolve_strand(id)?;
            http.get(&format!("{base}/api/v1/strands/{id}/errors?limit={limit}"))
                .await
        }
        Command::Strand(StrandCommand::Fork { id }) => {
            let id = defaults.resolve_strand(id)?;
            http.post(&format!("{base}/api/v1/strands/{id}/fork"), None)
                .await
        }
        Command::Strand(StrandCommand::Drive { id }) => {
            let id = defaults.resolve_strand(id)?;
            http.post(&format!("{base}/api/v1/strands/{id}/drive"), None)
                .await
        }
        Command::Strand(StrandCommand::Send {
            args,
            watch,
            watch_format,
        }) => {
            let (id, text) = split_send_args(args, defaults)?;
            let content = strand_send_body(text, defaults.soul());
            send(Request {
                client: &client,
                base: &base,
                strand: &id,
                body: content,
                watch,
                format: watch_format,
            })
            .await
        }
        Command::Strand(StrandCommand::Events { id, format }) => {
            let id = defaults.resolve_strand(id)?;
            http.follow(&format!("{base}/api/v1/strands/{id}/events"), format)
                .await
        }
        Command::Compact(CompactCommand::Exec {
            from,
            to,
            summary,
            summary_file,
        }) => {
            let strand = defaults.resolve_strand(None)?;
            let summary = match summary_file {
                Some(path) => read_summary_file(&path)?,
                None => summary.expect("clap requires summary or summary_file"),
            };
            let mut body = serde_json::json!({
                "from_message_id": from,
                "to_message_id": to,
                "summary": summary,
            });
            if let Some(soul) = defaults.soul() {
                body["soul_id"] = serde_json::Value::from(soul);
            }
            http.post(
                &format!("{base}/api/v1/strands/{strand}/compact"),
                Some(body),
            )
            .await
        }
        Command::Compact(CompactCommand::Capsule {
            from,
            to,
            from_seq,
            to_seq,
            summary,
            summary_file,
            source,
            reason,
            risk,
            queryability,
            dry_run,
        }) => {
            let body = compact_capsule_body(Capsule {
                from,
                to,
                start: from_seq,
                end: to_seq,
                summary,
                file: summary_file,
                source,
                reason,
                risk,
                queryability,
                preview: dry_run,
                soul: defaults.soul(),
            })?;
            let strand = defaults.resolve_strand(None)?;
            http.post(
                &format!("{base}/api/v1/strands/{strand}/compact"),
                Some(body),
            )
            .await
        }
        Command::Compact(CompactCommand::Query {
            compact_id,
            keyword,
            page_index,
            page_size,
        }) => {
            let mut url = format!(
                "{base}/api/v1/compacts/{compact_id}?page_index={page_index}&page_size={page_size}"
            );
            if let Some(keyword) = keyword.filter(|k| !k.is_empty()) {
                url.push_str(&format!("&keyword={}", urlencoding_encode(&keyword)));
            }
            http.get(&url).await
        }
    }
}

mod http;
use http::*;
