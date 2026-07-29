use std::collections::HashMap;
use std::fs;

use serde::Serialize;

use crate::config::Layout;
use crate::runtime::{self, Runtime};

#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    pub database: String,
    pub database_exists: bool,
    pub estate_bound: bool,
    pub estate_ready: bool,
    pub estate_error: Option<String>,
    pub genesis: String,
    pub memory_path: String,
    pub memory_present: bool,
    pub memory_readable: bool,
    pub memory_bytes: u64,
    pub provider: Option<ProviderDoctorReport>,
    pub ok: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderDoctorReport {
    pub profile: Option<String>,
    pub kind: Option<String>,
    pub model: Option<String>,
    pub bytes: Option<usize>,
    pub source: Option<String>,
    pub ok: bool,
    pub error: Option<String>,
}

pub async fn doctor() -> Result<DoctorReport, String> {
    let held = runtime::held();
    held.paths.configured(held).await
}

#[derive(Debug, Clone, Serialize)]
pub struct SeedReport {
    pub strand: String,
    pub accepted: bool,
    pub error: Option<santi_core::Fault>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuditRow {
    pub created_at: String,
    pub status: String,
    pub strand_id: String,
    pub turn_id: String,
    pub tool_name: String,
    pub arguments: serde_json::Value,
    pub output: Option<serde_json::Value>,
    pub error_text: Option<String>,
}

pub struct Audit<'a> {
    pub strand: Option<&'a str>,
    pub turn: Option<&'a str>,
    pub failed: bool,
    pub limit: usize,
    pub after: Option<&'a str>,
}

pub async fn inbox_seed(strand: &str, text: &str) -> Result<SeedReport, String> {
    runtime::held().paths.inbox_seed(strand, text).await
}

async fn inbox_seed_existing_strand(
    store: &santi_core::Store,
    strand: &str,
    text: &str,
) -> Result<SeedReport, String> {
    let inbox = santi_core::tag("inbox");
    let content = santi_core::message::Content::text(text);
    let source = santi_core::ingest::Source::new("offline_inbox_seed");
    store
        .accept_inbox(
            santi_core::InboxDraft {
                tag: &inbox,
                strand,
                kind: santi_core::message::Kind::SantiSystem,
                content: &content,
                source: Some(&source),
                created: &santi_core::now(),
            },
            500,
        )
        .await?;
    Ok(SeedReport {
        strand: strand.to_string(),
        accepted: true,
        error: None,
    })
}

impl Layout {
    pub async fn audit(&self, query: Audit<'_>) -> Result<Vec<AuditRow>, String> {
        let Audit {
            strand,
            turn,
            failed,
            limit,
            after,
        } = query;
        let store = santi_core::Store::open(&self.database).await?;
        let strands = match strand {
            Some(tag) => vec![
                store
                    .strand(tag)
                    .await?
                    .ok_or_else(|| format!("unknown strand: {tag}"))?,
            ],
            None => store.strands().await?,
        };
        let mut rows = Vec::new();
        for strand in strands {
            let replies = store
                .results(&strand.id)
                .await?
                .into_iter()
                .map(|reply| (reply.call.clone(), reply))
                .collect::<HashMap<_, _>>();
            for call in store.calls(&strand.id).await? {
                if turn.is_some_and(|turn| turn != call.turn)
                    || after.is_some_and(|after| call.created.as_str() <= after)
                {
                    continue;
                }
                let held = store
                    .turn(&call.turn)
                    .await?
                    .ok_or_else(|| format!("tool call turn {} missing", call.turn))?;
                let reply = replies.get(&call.id);
                let failed_call = held.status == santi_core::turn::Status::Failed
                    || reply.and_then(|reply| reply.error.as_ref()).is_some()
                    || reply
                        .and_then(|reply| reply.output.as_ref())
                        .and_then(|output| output.get("exit_code"))
                        .and_then(serde_json::Value::as_i64)
                        .is_some_and(|exit| exit != 0);
                if failed && !failed_call {
                    continue;
                }
                rows.push(AuditRow {
                    created_at: call.created,
                    status: match held.status {
                        santi_core::turn::Status::Running => "running",
                        santi_core::turn::Status::Completed => "completed",
                        santi_core::turn::Status::Failed => "failed",
                    }
                    .to_string(),
                    strand_id: strand.id.clone(),
                    turn_id: call.turn,
                    tool_name: call.tool,
                    arguments: call.arguments,
                    output: reply.and_then(|reply| reply.output.clone()),
                    error_text: reply.and_then(|reply| reply.error.clone()),
                });
            }
        }
        rows.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.turn_id.cmp(&right.turn_id))
        });
        if rows.len() > limit {
            rows.drain(..rows.len() - limit);
        }
        Ok(rows)
    }

    pub async fn doctor(&self) -> Result<DoctorReport, String> {
        self.doctor_report(None).await
    }

    pub async fn configured(&self, held: &Runtime) -> Result<DoctorReport, String> {
        let profile = Some(held.provider.clone());
        let provider = match held.resolved() {
            Ok(provider) => ProviderDoctorReport {
                profile,
                kind: Some(provider.kind().to_string()),
                model: Some(provider.model().to_string()),
                bytes: Some(provider.bytes()),
                source: Some("provider_config".to_string()),
                ok: true,
                error: None,
            },
            Err(error) => ProviderDoctorReport {
                profile,
                kind: None,
                model: None,
                bytes: None,
                source: None,
                ok: false,
                error: Some(error.to_string()),
            },
        };
        self.doctor_report(Some(provider)).await
    }

    async fn doctor_report(
        &self,
        provider: Option<ProviderDoctorReport>,
    ) -> Result<DoctorReport, String> {
        let database_exists = self.database.exists();
        let (estate_bound, estate_ready, estate_error) = if database_exists {
            match santi_core::Store::open(&self.database).await {
                Ok(store) => match store.soul(santi_core::GENESIS).await {
                    Ok(Some(_)) => (true, true, None),
                    Ok(None) => (true, false, Some("genesis soul is missing".to_string())),
                    Err(error) => (true, false, Some(error)),
                },
                Err(error) => (false, false, Some(error)),
            }
        } else {
            (false, false, Some("estate database is missing".to_string()))
        };
        let memory_path = self
            .runtime
            .join("souls")
            .join(santi_core::GENESIS)
            .join("memory")
            .join(santi_core::MEMORY);
        let memory_present = memory_path.exists();
        let (memory_readable, memory_bytes) = match fs::read(&memory_path) {
            Ok(bytes) => (true, bytes.len() as u64),
            Err(_) => (false, 0),
        };
        let memory_ok = !memory_present || memory_readable;
        let provider_ok = provider.as_ref().is_none_or(|provider| provider.ok);

        Ok(DoctorReport {
            database: self.database.display().to_string(),
            database_exists,
            estate_bound,
            estate_ready,
            estate_error,
            genesis: santi_core::GENESIS.to_string(),
            memory_path: memory_path.display().to_string(),
            memory_present,
            memory_readable,
            memory_bytes,
            provider,
            ok: estate_ready && memory_ok && provider_ok,
        })
    }

    pub async fn inbox_seed(&self, strand: &str, text: &str) -> Result<SeedReport, String> {
        let store = santi_core::Store::open(&self.database).await?;
        if store.strand(strand).await?.is_none() {
            return Err(format!("unknown strand: {strand}"));
        }
        inbox_seed_existing_strand(&store, strand, text).await
    }

    pub async fn inbox_seed_label(
        &self,
        soul: &str,
        label: &str,
        text: &str,
    ) -> Result<SeedReport, String> {
        let store = santi_core::Store::open(&self.database).await?;
        let strand = store.labeled(soul, label, &santi_core::now()).await?;
        inbox_seed_existing_strand(&store, &strand.id, text).await
    }
}
