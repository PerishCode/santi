use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use plumb::config::{Cascade, Listen};
use santi_api::config::{Profile, RuntimePaths, optional_env, santi_home};
use santi_api::runtime::Runtime;

#[derive(Debug, Cascade)]
pub struct SantiConfig {
    #[cascade(arg)]
    pub provider: String,
    #[cascade(section)]
    pub listen: Listen,
    #[cascade(section)]
    pub server: Server,
    #[cascade(section)]
    pub upgrade: Upgrade,
    #[cascade(section)]
    pub paths: Paths,
    #[cascade(section)]
    pub webhooks: Webhooks,
    pub providers: BTreeMap<String, Profile>,
}

impl Default for SantiConfig {
    fn default() -> Self {
        SantiConfig {
            provider: "openai".to_string(),
            listen: Listen {
                host: "127.0.0.1".to_string(),
                port: 43307,
                prefix: String::new(),
            },
            server: Server::default(),
            upgrade: Upgrade::default(),
            paths: Paths::default(),
            webhooks: Webhooks::default(),
            providers: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Cascade)]
#[cascade(section)]
pub struct Server {
    pub shutdown_grace_secs: u64,
}

impl Default for Server {
    fn default() -> Self {
        Server {
            shutdown_grace_secs: 600,
        }
    }
}

#[derive(Debug, Cascade)]
#[cascade(section)]
pub struct Upgrade {
    pub timeout_secs: u64,
    pub finalizer_bin: PathBuf,
    pub soul: Option<String>,
    pub strand: Option<String>,
}

impl Default for Upgrade {
    fn default() -> Self {
        Upgrade {
            timeout_secs: 600,
            finalizer_bin: PathBuf::from("/usr/bin/santi"),
            soul: None,
            strand: None,
        }
    }
}

#[derive(Debug, Default, Cascade)]
#[cascade(section)]
pub struct Paths {
    pub database: Option<PathBuf>,
    pub runtime_root: Option<PathBuf>,
    pub execution_root: Option<PathBuf>,
    pub constitution_file: Option<PathBuf>,
}

#[derive(Debug, Default, Cascade)]
#[cascade(section)]
pub struct Webhooks {
    #[cascade(section)]
    pub github: Github,
    #[cascade(section)]
    pub feishu: Feishu,
}

#[derive(Debug, Default, Cascade)]
#[cascade(section)]
pub struct Github {
    pub self_login: Option<String>,
    pub allow: Option<String>,
}

#[derive(Debug, Default, Cascade)]
#[cascade(section)]
pub struct Feishu {
    pub encrypt_key: Option<String>,
    pub allow: Option<String>,
}

pub fn config_path(over: Option<&str>) -> PathBuf {
    over.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| optional_env("SANTI_CONFIG").map(PathBuf::from))
        .unwrap_or_else(|| santi_home().join("santi.toml"))
}

pub fn boot(config: Option<&str>, over: SantiConfigPartial) -> Result<(), String> {
    let path = config_path(config);
    let file = path.is_file().then_some(path.as_path());
    let held = SantiConfig::resolve_with(file, over).map_err(|error| error.to_string())?;
    santi_api::runtime::hold(runtime(held));
    Ok(())
}

fn runtime(held: SantiConfig) -> Runtime {
    let home = santi_home();
    Runtime {
        bind: held.listen.address(),
        listen_port: held.listen.port,
        provider: held.provider,
        providers: held.providers,
        paths: RuntimePaths {
            database_path: held
                .paths
                .database
                .unwrap_or_else(|| home.join("runtime").join("db")),
            runtime_root: held
                .paths
                .runtime_root
                .unwrap_or_else(|| home.join("runtime")),
            execution_root: held
                .paths
                .execution_root
                .unwrap_or_else(|| home.join("execution")),
        },
        shutdown_grace: Duration::from_secs(held.server.shutdown_grace_secs),
        upgrade_timeout: Duration::from_secs(held.upgrade.timeout_secs),
        finalizer_bin: held.upgrade.finalizer_bin,
        handover_soul: held.upgrade.soul,
        handover_strand: held.upgrade.strand,
        github_login: held.webhooks.github.self_login,
        github_allow: held.webhooks.github.allow,
        feishu_key: held.webhooks.feishu.encrypt_key,
        feishu_allow: held.webhooks.feishu.allow,
        constitution: held.paths.constitution_file,
    }
}

#[derive(clap::Parser)]
#[command(disable_help_subcommand = true)]
pub struct ServiceCli {
    #[command(subcommand)]
    pub command: Option<ServiceCommand>,
    #[arg(long, global = true)]
    pub config: Option<String>,
    #[command(flatten)]
    pub over: SantiConfigArgs,
}

#[derive(Clone, Copy, clap::Subcommand)]
pub enum ServiceCommand {
    Serve,
    #[command(name = "export-openapi")]
    ExportOpenApi,
}
