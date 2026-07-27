use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::json;
use sha2::{Digest, Sha256};

use super::Service;
use crate::store::{JobAttention, JobRecord, Notice};
use crate::{ingest, message, now, stamped};

const NUMERATOR: u128 = 4;
const DENOMINATOR: u128 = 5;

pub(super) fn capture(service: &Service, record: JobRecord) -> Result<(), String> {
    if record.job.state.terminal() {
        return Ok(());
    }
    let Some(started) = record.started else {
        return Ok(());
    };
    let current = epoch()?;
    let elapsed = current.saturating_sub(started) as u64;
    let (stdout, stderr) = output(service, &record)?;
    let combined = stdout.saturating_add(stderr);
    let runtime =
        !record.runtime && threshold(elapsed, record.job.timeout_seconds.saturating_mul(1000));
    let out = !record.output && threshold(combined, record.job.output_limit_bytes);
    let (reminded, tick, next) = reminder(&record, elapsed, started)?;
    if !runtime && !out && !reminded {
        return Ok(());
    }

    let observed = now();
    let revision = record
        .revision
        .checked_add(1)
        .ok_or_else(|| "job attention revision is out of range".to_string())?;
    let causes = causes(runtime, out, reminded);
    let content = fragment(Fragment {
        record: &record,
        observed: &observed,
        elapsed,
        stdout,
        stderr,
        next: next.as_deref(),
    });
    let encoded = serde_json::to_vec(&content).map_err(|error| error.to_string())?;
    let digest = format!("{:x}", Sha256::digest(encoded));
    let key = format!("job/{}/{}", record.job.id, record.stamp);
    let source = ingest::Source::new("job")
        .with_ref(record.job.id.clone())
        .with_metadata(json!({
            "schema": "santi.job.attention.v1",
            "stamp": record.stamp,
            "revision": revision,
            "observed_at": observed,
        }));
    service.notify(
        JobAttention {
            id: &record.job.id,
            base: record.revision,
            at: &observed,
            runtime,
            output: out,
            reminded,
            tick,
            next: next.as_deref(),
        },
        Notice {
            strand: &record.job.origin.strand,
            key: &key,
            revision,
            digest: &digest,
            content,
            source,
            causes,
        },
    )
}

struct Fragment<'a> {
    record: &'a JobRecord,
    observed: &'a str,
    elapsed: u64,
    stdout: u64,
    stderr: u64,
    next: Option<&'a str>,
}

fn fragment(input: Fragment<'_>) -> message::Content {
    let job = &input.record.job;
    message::Content::text(
        [
            "item_kind: job_attention".to_string(),
            format!("job_id: {}", job.id),
            format!("stamp: {}", input.record.stamp),
            format!("description: {:?}", job.description),
            format!("state: {}", job.state.encode()),
            format!("observed_at: {}", input.observed),
            format!("elapsed_seconds: {}", input.elapsed / 1000),
            format!("timeout_seconds: {}", job.timeout_seconds),
            format!("stdout_bytes: {}", input.stdout),
            format!("stderr_bytes: {}", input.stderr),
            format!(
                "combined_output_bytes: {}",
                input.stdout.saturating_add(input.stderr)
            ),
            format!("output_limit_bytes: {}", job.output_limit_bytes),
            format!("next_reminder_at: {}", input.next.unwrap_or("none")),
        ]
        .join("\n"),
    )
}

fn reminder(
    record: &JobRecord,
    elapsed: u64,
    started: i64,
) -> Result<(bool, u64, Option<String>), String> {
    let Some(interval) = record.job.remind else {
        return Ok((false, record.reminder, None));
    };
    let period = interval
        .checked_mul(1000)
        .ok_or_else(|| "job reminder interval is out of range".to_string())?;
    let tick = elapsed / period;
    if tick == 0 || tick <= record.reminder {
        return Ok((false, record.reminder, record.job.next.clone()));
    }
    let following = tick
        .checked_add(1)
        .and_then(|tick| tick.checked_mul(interval))
        .ok_or_else(|| "job reminder cadence is out of range".to_string())?;
    Ok((true, tick, Some(future(started, following)?)))
}

fn causes(runtime: bool, output: bool, reminder: bool) -> Vec<String> {
    let mut causes = Vec::new();
    if runtime {
        causes.push("runtime_threshold".to_string());
    }
    if output {
        causes.push("output_threshold".to_string());
    }
    if reminder {
        causes.push("reminder".to_string());
    }
    causes
}

fn threshold(value: u64, limit: u64) -> bool {
    u128::from(value) * DENOMINATOR >= u128::from(limit) * NUMERATOR
}

fn output(service: &Service, record: &JobRecord) -> Result<(u64, u64), String> {
    let directory = service.jobhome(record);
    Ok((
        size(&directory.join("stdout.log"))?,
        size(&directory.join("stderr.log"))?,
    ))
}

fn size(path: &std::path::Path) -> Result<u64, String> {
    match std::fs::metadata(path) {
        Ok(metadata) => Ok(metadata.len()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error.to_string()),
    }
}

fn epoch() -> Result<i64, String> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?;
    i64::try_from(elapsed.as_millis()).map_err(|_| "system clock is out of range".to_string())
}

fn future(millis: i64, seconds: u64) -> Result<String, String> {
    let base = u64::try_from(millis).map_err(|_| "job start time is out of range".to_string())?;
    let total = base
        .checked_add(
            seconds
                .checked_mul(1000)
                .ok_or_else(|| "job reminder cadence is out of range".to_string())?,
        )
        .ok_or_else(|| "job reminder cadence is out of range".to_string())?;
    stamped(UNIX_EPOCH + Duration::from_millis(total))
}
