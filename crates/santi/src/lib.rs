pub mod auth;
pub mod cli;
pub mod client;
pub mod config;
mod text;
pub mod watch;

use anyhow::Result;
use clap::Parser;

use auth::{Credentials, resolve_edge_bearer};
use cli::{Cli, ClientDefaults};
use client::run_client;

pub async fn run() -> Result<()> {
    config::load();
    let cli = Cli::parse();
    let defaults = ClientDefaults {
        strand: cli.strand,
        soul: cli.soul,
    };
    let bearer = resolve_edge_bearer(Credentials {
        endpoint: cli.auth_token_url.as_deref(),
        identity: cli.auth_client_id.as_deref(),
        username: cli.auth_username.as_deref(),
        password: cli.auth_password.as_deref(),
        key: cli.api_key.as_deref(),
    })
    .await?;
    run_client(&cli.base_url, bearer.as_deref(), &defaults, cli.command).await
}
