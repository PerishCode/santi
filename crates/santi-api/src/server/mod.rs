mod effects;
mod error;
mod errors;
mod ingress;
mod openapi;
mod routes;
mod sse;

use santi_core::service::{self, Service};
use std::{env, fs, net::SocketAddr};

use crate::{config, provider};

pub use effects::{ResolveEffectRequest, effect_status, resolve_effect};
pub use error::ApiError;
pub use routes::{drive_strand, health, receipt_status, send_strand};

pub fn export_openapi_json() -> Result<String, String> {
    serde_json::to_string_pretty(&openapi::document()).map_err(|error| error.to_string())
}

pub async fn serve(config: config::ConfigService) -> Result<(), String> {
    let provider = provider::from_config(config.provider_config()?);
    let bind = bind_addr_string(&config.listen()?);
    let paths = config::resolve_runtime_paths();
    if let Some(parent) = paths.database_path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::create_dir_all(&paths.runtime_root).map_err(|error| error.to_string())?;
    fs::create_dir_all(&paths.execution_root).map_err(|error| error.to_string())?;
    let service = Service::open(
        service::Config {
            database_path: paths.database_path.display().to_string(),
            runtime_root: paths.runtime_root.display().to_string(),
            execution_root: paths.execution_root.display().to_string(),
            bind_addr: Some(bind.clone()),
        },
        provider,
    )?;
    crate::upgrade::register_attempt_handover_budgets(&service)?;
    let address: SocketAddr = bind
        .parse()
        .map_err(|_| "SANTI_HOST/SANTI_PORT did not form a valid socket address".to_string())?;
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .map_err(|error| error.to_string())?;
    service.resume_pending()?;
    println!("santi-api listening on http://{address}");
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

fn shutdown_grace() -> std::time::Duration {
    let secs = env::var("SANTI_SHUTDOWN_GRACE_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(600);
    std::time::Duration::from_secs(secs)
}

fn bind_addr_string(listen: &plumb_lib::config::Listen) -> String {
    let host = env::var("SANTI_HOST").unwrap_or_else(|_| listen.host.clone());
    let port = env::var("SANTI_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(listen.port);
    format!("{host}:{port}")
}
