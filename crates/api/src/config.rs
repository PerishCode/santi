use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use plumb::config::{Cascade, Listen};
use santi_api::config::{Layout, Profile, env, home};
use santi_api::runtime::{self, Runtime};

pub fn load() {
    dotenvy::dotenv().ok();
}

#[derive(Debug, Cascade)]
pub struct Config {
    #[cascade(arg)]
    pub provider: String,
    #[cascade(section)]
    pub listen: Listen,
    #[cascade(section)]
    pub server: Server,
    #[cascade(section)]
    pub jobs: Jobs,
    #[cascade(section)]
    pub paths: Paths,
    #[cascade(section)]
    pub webhooks: Webhooks,
    #[cascade(section)]
    pub capability: Capability,
    pub environment: BTreeMap<String, String>,
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
            jobs: Jobs::default(),
            paths: Paths::default(),
            webhooks: Webhooks::default(),
            capability: Capability::default(),
            environment: BTreeMap::new(),
            providers: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Cascade)]
#[cascade(section)]
pub struct Jobs {
    pub acknowledged_retention_seconds: u64,
}

impl Default for Jobs {
    fn default() -> Self {
        Self {
            acknowledged_retention_seconds: santi_api::RETENTION,
        }
    }
}

impl Jobs {
    pub fn retention(&self) -> Result<Duration, String> {
        if self.acknowledged_retention_seconds == 0 {
            return Err("acknowledged job retention must be greater than zero".to_string());
        }
        Ok(Duration::from_secs(self.acknowledged_retention_seconds))
    }
}

#[derive(Debug, Cascade)]
#[cascade(section)]
pub struct Server {
    pub grace: u64,
}

impl Default for Server {
    fn default() -> Self {
        Server { grace: 30 }
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

#[derive(Cascade)]
#[cascade(section)]
pub struct Capability {
    pub issuer: String,
    pub audience: String,
    pub key_id: String,
    pub private_key: String,
    pub ttl_seconds: u64,
}

impl Default for Capability {
    fn default() -> Self {
        Self {
            issuer: String::new(),
            audience: String::new(),
            key_id: String::new(),
            private_key: String::new(),
            ttl_seconds: 120,
        }
    }
}

impl std::fmt::Debug for Capability {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Capability")
            .field("issuer", &self.issuer)
            .field("audience", &self.audience)
            .field("key_id", &self.key_id)
            .field("private_key", &"[redacted]")
            .field("ttl_seconds", &self.ttl_seconds)
            .finish()
    }
}

impl Capability {
    fn issuer(&self) -> Result<Option<santi_core::capability::Issuer>, String> {
        let configured = [
            self.issuer.as_str(),
            self.audience.as_str(),
            self.key_id.as_str(),
            self.private_key.as_str(),
        ]
        .iter()
        .any(|value| !value.trim().is_empty());
        if !configured {
            return Ok(None);
        }
        santi_core::capability::Issuer::new(
            &self.issuer,
            &self.audience,
            santi_core::capability::Key {
                id: &self.key_id,
                private: &self.private_key,
            },
            self.ttl_seconds,
        )
        .map(Some)
    }
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
    let held = resolved(file, over).map_err(|error| error.to_string())?;
    runtime::hold(runtime(held)?);
    Ok(())
}

fn resolved(
    file: Option<&std::path::Path>,
    over: ConfigPartial,
) -> Result<Config, plumb::config::Error> {
    let mut held = Config::default();
    if let Some(path) = file {
        held = held.merge(plumb::config::load::<ConfigPartial>(path)?);
    }
    held = held.merge(Config::lookup("SANTI", &legacy)?);
    held = held.merge(Config::env("SANTI")?);
    Ok(held.merge(over))
}

fn legacy(key: &str) -> Option<String> {
    let names: &[&str] = match key {
        "SANTI_LISTEN_HOST" => &["SANTI_HOST"],
        "SANTI_LISTEN_PORT" => &["SANTI_PORT"],
        "SANTI_PATHS_DATABASE" => &["SANTI_DB"],
        "SANTI_PATHS_RUNTIME" => &["SANTI_PATHS_RUNTIME_ROOT", "SANTI_RUNTIME_ROOT"],
        "SANTI_PATHS_EXECUTION" => &["SANTI_PATHS_EXECUTION_ROOT", "SANTI_EXECUTION_ROOT"],
        "SANTI_SERVER_GRACE" => &[
            "SANTI_SERVER_SHUTDOWN_GRACE_SECS",
            "SANTI_SHUTDOWN_GRACE_SECS",
        ],
        "SANTI_WEBHOOKS_GITHUB_LOGIN" => &["SANTI_WEBHOOK_GITHUB_LOGIN"],
        "SANTI_WEBHOOKS_GITHUB_ALLOW" => &["SANTI_WEBHOOK_GITHUB_ALLOW"],
        "SANTI_WEBHOOKS_FEISHU_ENCRYPT_KEY" => &["SANTI_WEBHOOK_FEISHU_ENCRYPT_KEY"],
        "SANTI_WEBHOOKS_FEISHU_ALLOW" => &["SANTI_WEBHOOK_FEISHU_ALLOW"],
        _ => &[],
    };
    names.iter().find_map(|name| env(name))
}

fn runtime(held: Config) -> Result<Runtime, String> {
    let home = home();
    Ok(Runtime {
        bind: held.listen.address(),
        port: held.listen.port,
        provider: held.provider,
        providers: held.providers,
        environment: held.environment,
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
        retention: held.jobs.retention()?,
        github: runtime::Github {
            login: held.webhooks.github.login,
            allow: held.webhooks.github.allow,
        },
        feishu: runtime::Feishu {
            secret: held.webhooks.feishu.encrypt_key,
            allow: held.webhooks.feishu.allow,
        },
        capability: held.capability.issuer()?,
        constitution: held.paths.charter,
    })
}
