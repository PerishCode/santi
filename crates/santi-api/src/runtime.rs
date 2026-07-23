use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use crate::config::{Profile, ProviderConfig, RuntimePaths, resolve_provider_config};

#[derive(Debug)]
pub struct Runtime {
    pub bind: String,
    pub listen_port: u16,
    pub provider: String,
    pub providers: BTreeMap<String, Profile>,
    pub paths: RuntimePaths,
    pub shutdown_grace: Duration,
    pub upgrade_timeout: Duration,
    pub finalizer_bin: PathBuf,
    pub handover_soul: Option<String>,
    pub handover_strand: Option<String>,
    pub github_login: Option<String>,
    pub github_allow: Option<String>,
    pub feishu_key: Option<String>,
    pub feishu_allow: Option<String>,
    pub constitution: Option<PathBuf>,
}

impl Runtime {
    pub fn provider_config(&self) -> Result<ProviderConfig, String> {
        let profile = self
            .providers
            .get(&self.provider)
            .ok_or_else(|| format!("provider {} is not defined in the config", self.provider))?;
        resolve_provider_config(&self.provider, profile)
    }
}

static HELD: OnceLock<Runtime> = OnceLock::new();

pub fn hold(runtime: Runtime) {
    let _ = HELD.set(runtime);
}

pub fn held() -> &'static Runtime {
    HELD.get()
        .expect("the runtime is resolved once at boot, before any use")
}
