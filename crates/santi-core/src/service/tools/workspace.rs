#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::{path::PathBuf, process::Stdio};

use super::{Origin, Service, shell};
use crate::{parsed, workspace};

impl Service {
    pub(super) async fn prepared(
        &self,
        origin: Origin<'_>,
        args: shell::Args,
    ) -> Result<shell::Prepared, String> {
        std::fs::create_dir_all(self.soulhome(origin.soul)).map_err(|error| error.to_string())?;
        std::fs::create_dir_all(self.strandhome(origin.strand))
            .map_err(|error| error.to_string())?;
        let cwd = self.situated(origin.strand, origin.soul, args.cwd.as_deref())?;
        std::fs::create_dir_all(&cwd).map_err(|error| error.to_string())?;
        let capability = self
            .permit(origin.strand, origin.turn, origin.call, origin.effect)
            .await?;
        let mut command = shell::shell(&args.command);
        #[cfg(unix)]
        command.process_group(0);
        command
            .current_dir(&cwd)
            .env("SANTI_SOUL_MEMORY_DIR", self.soulhome(origin.soul))
            .env("SANTI_STRAND_MEMORY_DIR", self.strandhome(origin.strand))
            .env("SANTI_SOUL_ID", origin.soul)
            .env("SANTI_STRAND_ID", origin.strand)
            .env("SANTI_TURN_ID", origin.turn)
            .env("SANTI_TOOL_CALL_ID", origin.call)
            .env("SANTI_EFFECT_ID", origin.effect)
            .env("SANTI_JOB_CREATE_CAPABILITY", capability)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        Ok(shell::Prepared { command, cwd })
    }

    pub(in crate::service) fn situated(
        &self,
        strand: &str,
        soul: &str,
        cwd: Option<&str>,
    ) -> Result<PathBuf, String> {
        let Some(cwd) = cwd else {
            return Ok(self.execution());
        };
        let uri = parsed(cwd)?;
        let root = match uri.root {
            workspace::Root::Soul => self.soulhome(soul),
            workspace::Root::Strand => self.strandhome(strand),
        };
        Ok(root.join(uri.path))
    }

    pub(in crate::service) fn runtime(&self) -> PathBuf {
        PathBuf::from(&self.config.runtime)
    }

    pub(in crate::service) fn execution(&self) -> PathBuf {
        PathBuf::from(&self.config.execution)
    }

    pub(in crate::service) fn soulhome(&self, soul: &str) -> PathBuf {
        self.runtime().join("souls").join(soul).join("memory")
    }

    pub(in crate::service) fn memoir(&self, soul: &str) -> PathBuf {
        self.runtime()
            .join("souls")
            .join(soul)
            .join("memory")
            .join(crate::workspace::MEMORY)
    }

    pub(in crate::service) fn strandhome(&self, strand: &str) -> PathBuf {
        self.runtime().join("strands").join(strand).join("memory")
    }

    pub(in crate::service) fn journal(&self, strand: &str) -> PathBuf {
        self.strandhome(strand).join("MEMORY.md")
    }

    pub(in crate::service) fn charter(&self) -> PathBuf {
        self.config
            .constitution
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| self.runtime().join("constitution.md"))
    }
}
