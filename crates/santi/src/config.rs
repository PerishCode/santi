use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use plumb::config::{Cascade, Listen};
use santi_api::config::{Layout, Profile, env, home};
use santi_api::runtime::{self, Runtime};

#[derive(Debug, Cascade)]
pub struct Config {
    #[cascade(arg)]
    pub provider: String,
    #[cascade(section)]
    pub listen: Listen,
    #[cascade(section)]
    pub server: Server,
    #[cascade(section)]
    pub paths: Paths,
    #[cascade(section)]
    pub webhooks: Webhooks,
    pub providers: BTreeMap<String, Profile>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            provider: "openai".to_string(),
            listen: Listen {
                host: "127.0.0.1".to_string(),
                port: 43307,
                prefix: String::new(),
            },
            server: Server::default(),
            paths: Paths::default(),
            webhooks: Webhooks::default(),
            providers: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Cascade)]
#[cascade(section)]
pub struct Server {
    pub grace: u64,
}

impl Default for Server {
    fn default() -> Self {
        Server { grace: 600 }
    }
}

#[derive(Debug, Default, Cascade)]
#[cascade(section)]
pub struct Paths {
    pub database: Option<PathBuf>,
    pub runtime: Option<PathBuf>,
    pub execution: Option<PathBuf>,
    pub charter: Option<PathBuf>,
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
    pub login: Option<String>,
    pub allow: Option<String>,
}

#[derive(Debug, Default, Cascade)]
#[cascade(section)]
pub struct Feishu {
    pub encrypt_key: Option<String>,
    pub allow: Option<String>,
}

pub fn path(over: Option<&str>) -> PathBuf {
    over.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| env("SANTI_CONFIG").map(PathBuf::from))
        .unwrap_or_else(|| home().join("santi.toml"))
}

pub fn boot(config: Option<&str>, over: ConfigPartial) -> Result<(), String> {
    let file = path(config);
    let file = file.is_file().then_some(file.as_path());
    let held = Config::resolve_with(file, over).map_err(|error| error.to_string())?;
    runtime::hold(runtime(held));
    Ok(())
}

fn runtime(held: Config) -> Runtime {
    let home = home();
    Runtime {
        bind: held.listen.address(),
        port: held.listen.port,
        provider: held.provider,
        providers: held.providers,
        paths: Layout {
            database: held
                .paths
                .database
                .unwrap_or_else(|| home.join("runtime").join("db")),
            runtime: held.paths.runtime.unwrap_or_else(|| home.join("runtime")),
            execution: held
                .paths
                .execution
                .unwrap_or_else(|| home.join("execution")),
        },
        grace: Duration::from_secs(held.server.grace),
        github: runtime::Github {
            login: held.webhooks.github.login,
            allow: held.webhooks.github.allow,
        },
        feishu: runtime::Feishu {
            secret: held.webhooks.feishu.encrypt_key,
            allow: held.webhooks.feishu.allow,
        },
        constitution: held.paths.charter,
    }
}

#[derive(clap::Parser)]
#[command(disable_help_subcommand = true)]
pub struct Service {
    #[command(subcommand)]
    pub command: Option<Mode>,
    #[arg(long, global = true)]
    pub config: Option<String>,
    #[command(flatten)]
    pub over: ConfigArgs,
}

#[derive(Clone, Copy, clap::Subcommand)]
pub enum Mode {
    Serve,
    #[command(name = "export-openapi")]
    Export,
}

pub fn shelter() -> PathBuf {
    env("HOME")
        .map(|home| PathBuf::from(home).join(".cache/santi"))
        .unwrap_or_else(std::env::temp_dir)
}
