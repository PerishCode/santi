use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use crate::config::{Layout, Profile, Resolved};

#[derive(Debug)]
pub struct Runtime {
    pub bind: String,
    pub port: u16,
    pub provider: String,
    pub providers: BTreeMap<String, Profile>,
    pub environment: BTreeMap<String, String>,
    pub paths: Layout,
    pub grace: Duration,
    pub retention: Duration,
    pub github: Github,
    pub feishu: Feishu,
    pub constitution: Option<PathBuf>,
}

#[derive(Debug, Default)]
pub struct Github {
    pub login: Option<String>,
    pub allow: Option<String>,
}

#[derive(Debug, Default)]
pub struct Feishu {
    pub secret: Option<String>,
    pub allow: Option<String>,
}

impl Runtime {
    pub fn resolved(&self) -> Result<Resolved, String> {
        let profile = self
            .providers
            .get(&self.provider)
            .ok_or_else(|| format!("provider {} is not defined in the config", self.provider))?;
        profile.resolve(&self.provider)
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
