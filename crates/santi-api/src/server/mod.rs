mod effects;
mod error;
mod errors;
mod ingress;
mod jobs;
mod openapi;
mod routes;
mod sse;

use santi_core::service::{self, Service};
use std::{fs, net::SocketAddr};

use crate::provider;

pub use effects::{ResolveEffectRequest, effect, settle};
pub use error::ApiError;
pub use jobs::{
    CreateJobRequest, acknowledge as acknowledge_job, cancel as cancel_job, create as create_job,
    get as get_job, list as list_jobs, logs as job_logs,
};
pub use routes::{drive, health, receipt, send};
pub use routes::{
    end_soul_environ as end_soul_environ_handler, end_strand_environ as end_strand_environ_handler,
    set_soul_environ as set_soul_environ_handler, set_strand_environ as set_strand_environ_handler,
    soul_environs as soul_environs_handler, strand_environs as strand_environs_handler,
};

pub fn export_openapi_json() -> Result<String, String> {
    serde_json::to_string_pretty(&openapi::document()).map_err(|error| error.to_string())
}

pub async fn serve() -> Result<(), String> {
    let held = crate::runtime::held();
    let provider = provider::build(held.resolved()?);
    let bind = held.bind.clone();
    let paths = held.paths.clone();
    if let Some(parent) = paths.database.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::create_dir_all(&paths.runtime).map_err(|error| error.to_string())?;
    fs::create_dir_all(&paths.execution).map_err(|error| error.to_string())?;
    let supervisor = std::sync::Arc::new(crate::jobs::Native::current()?);
    let service = Service::supervised(
        service::Config {
            database: paths.database.display().to_string(),
            runtime: paths.runtime.display().to_string(),
            execution: paths.execution.display().to_string(),
            bind: Some(bind.clone()),
            constitution: held
                .constitution
                .as_ref()
                .map(|path| path.display().to_string()),
            environment: held.environment.clone(),
        },
        provider,
        supervisor,
    )
    .await?
    .retain(held.retention)?;
    let address: SocketAddr = bind
        .parse()
        .map_err(|_| "listen host/port did not form a valid socket address".to_string())?;
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .map_err(|error| error.to_string())?;
    service.resume().await?;
    let watcher = {
        let service = service.clone();
        tokio::spawn(async move { service.watch().await })
    };
    println!("santi-api listening on http://{address}");
    let shutdown_signal = {
        let service = service.clone();
        let grace = held.grace;
        async move {
            wait_for_shutdown_signal().await;
            println!(
                "santi-api: shutdown signal received — quiescing (no new turns; grace={}s)",
                grace.as_secs()
            );
            service.quiesce(grace);
        }
    };
    let drainer = service.clone();
    let result = axum::serve(listener, routes::router(service))
        .with_graceful_shutdown(shutdown_signal)
        .await
        .map_err(|error| error.to_string());
    if !drainer.closing() {
        drainer.quiesce(std::time::Duration::ZERO);
    }
    watcher.await.map_err(|error| error.to_string())?;
    result?;
    drainer.drain().await;
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
