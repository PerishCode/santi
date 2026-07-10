mod error;
mod ingress;
mod openapi;
mod routes;
mod sse;

use std::{env, fs, net::SocketAddr};

use crate::{config, provider};
use santi_core::{SantiService, SantiServiceConfig};

pub use error::ApiError;
pub use routes::send_strand;

pub fn export_openapi_json() -> Result<String, String> {
    serde_json::to_string_pretty(&openapi::document()).map_err(|error| error.to_string())
}

pub async fn serve(config: config::ConfigService) -> Result<(), String> {
    let provider = provider::from_config(config.provider_config()?);
    // Paths anchor on the santi home (`SANTI_HOME`, else `~/.santi`); explicit env
    // always overrides (see `resolve_runtime_paths`). The data dirs are created
    // here so a zero-config run works (the offline ops paths only read).
    let paths = config::resolve_runtime_paths();
    if let Some(parent) = paths.database_path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::create_dir_all(&paths.runtime_root).map_err(|error| error.to_string())?;
    fs::create_dir_all(&paths.execution_root).map_err(|error| error.to_string())?;
    let service = SantiService::open(
        SantiServiceConfig {
            database_path: paths.database_path.display().to_string(),
            runtime_root: paths.runtime_root.display().to_string(),
            execution_root: paths.execution_root.display().to_string(),
            bind_addr: Some(bind_addr_string()),
        },
        provider,
    )?;
    let address: SocketAddr = bind_addr_string()
        .parse()
        .map_err(|_| "SANTI_HOST/SANTI_PORT did not form a valid socket address".to_string())?;
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .map_err(|error| error.to_string())?;
    // santi carries NO auth of its own: access control lives entirely at the edge
    // (authentik forward-auth in front of the window nginx; on-box callers reach
    // 127.0.0.1 directly). The only in-process gate is webhook signature
    // verification (per subscription), which is independent of this and untouched.
    // Liveness: re-drive any requests stranded by a previous crash.
    service.resume_pending();
    println!("santi-api listening on http://{address}");
    // Graceful shutdown (PHASE-07): on SIGTERM/Ctrl-C, latch the service so no
    // new turns start (inbox consumption pauses; ingest still enqueues durably),
    // let axum drain in-flight HTTP, then wait out the in-flight turn before
    // exiting. The external upgrade flow owns the hard bound (SIGKILL after its
    // timeout); this is the cooperative half.
    let shutdown_signal = {
        let service = service.clone();
        async move {
            wait_for_shutdown_signal().await;
            println!("santi-api: shutdown signal received — quiescing (no new turns)");
            service.begin_shutdown();
        }
    };
    let drainer = service.clone();
    axum::serve(listener, routes::router(service))
        .with_graceful_shutdown(shutdown_signal)
        .await
        .map_err(|error| error.to_string())?;
    drainer.drain_running_turns(shutdown_grace()).await;
    println!("santi-api: drained; exiting");
    Ok(())
}

/// Resolve on the shutdown signal: SIGTERM (systemd/`systemctl stop`) or Ctrl-C.
async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut term = match signal(SignalKind::terminate()) {
            Ok(term) => term,
            Err(error) => {
                eprintln!("santi-api: cannot install SIGTERM handler: {error}");
                return;
            }
        };
        tokio::select! {
            _ = term.recv() => {}
            _ = tokio::signal::ctrl_c() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

/// How long the service waits for the in-flight turn to finish on shutdown.
/// `SANTI_SHUTDOWN_GRACE_SECS`, default 600s (turns can run minutes). The systemd
/// unit's `TimeoutStopSec` must be at least this so systemd does not SIGKILL first.
fn shutdown_grace() -> std::time::Duration {
    let secs = env::var("SANTI_SHUTDOWN_GRACE_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(600);
    std::time::Duration::from_secs(secs)
}

fn bind_addr_string() -> String {
    let host = env::var("SANTI_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port = env::var("SANTI_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(43307);
    format!("{host}:{port}")
}
