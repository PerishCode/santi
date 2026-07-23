use super::*;

pub(super) struct Host {
    paths: RuntimePaths,
    backup: PathBuf,
    store: Store,
    candidate: Artifact,
    previous: Artifact,
}

impl Host {
    pub(super) fn new(
        paths: RuntimePaths,
        store: Store,
        candidate: Artifact,
        previous: Artifact,
    ) -> Self {
        let backup = paths
            .runtime_root
            .with_file_name("santi-runtime-backup.tar.gz");
        Self {
            paths,
            backup,
            store,
            candidate,
            previous,
        }
    }

    fn privileged(&self, args: &[&str]) -> Result<(), String> {
        let status = Command::new("sudo")
            .arg("-n")
            .args(args)
            .status()
            .map_err(|error| format!("sudo -n {}: {error}", args.join(" ")))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("sudo -n {} failed", args.join(" ")))
        }
    }

    fn systemctl(&self, action: &str) -> Result<(), String> {
        self.privileged(&["systemctl", action, SANTI_SERVICE])
    }
}

fn probe_readiness(binary: &Path) -> Result<Option<UpgradeReadiness>, String> {
    probe_final_version_storage(binary)?;
    probe_runtime_readiness(binary)
}

impl UpgradeHost for Host {
    fn graceful_stop(&mut self, _grace: Duration) -> Result<(), String> {
        self.systemctl("stop")
    }

    fn snapshot(&mut self) -> Result<(), String> {
        let root = &self.paths.runtime_root;
        let parent = root.parent().ok_or("runtime_root has no parent")?;
        let name = root.file_name().ok_or("runtime_root has no name")?;
        let status = Command::new("tar")
            .arg("czf")
            .arg(&self.backup)
            .arg("-C")
            .arg(parent)
            .arg(name)
            .status()
            .map_err(|error| format!("tar snapshot: {error}"))?;
        if status.success() {
            Ok(())
        } else {
            Err("runtime snapshot (tar) failed".to_string())
        }
    }

    fn install(&mut self, deb: &str) -> Result<(), String> {
        self.privileged(&["dpkg", "-i", deb])
    }

    fn trial_probe(&mut self) -> Result<UpgradeReadiness, String> {
        self.systemctl("start")?;
        let deadline = Instant::now() + upgrade_timeout();
        let mut last_detail = "service health was not reachable".to_string();
        let binary = final_version_binary();
        let readiness = loop {
            match probe_readiness(&binary) {
                Ok(Some(readiness)) => break Ok(readiness),
                Ok(None) => {}
                Err(error) => last_detail = error,
            }
            if Instant::now() >= deadline {
                break Err(format!("trial probe timed out: {last_detail}"));
            }
            thread::sleep(Duration::from_millis(500));
        };
        let stop = self.systemctl("stop");
        match (readiness, stop) {
            (Ok(readiness), Ok(())) => Ok(readiness),
            (Err(error), Ok(())) => Err(error),
            (Ok(_), Err(error)) => Err(format!("trial service stop failed: {error}")),
            (Err(probe_error), Err(stop_error)) => Err(format!(
                "{probe_error}; trial service stop also failed: {stop_error}"
            )),
        }
    }

    fn retain_candidate(&mut self) -> Result<(), String> {
        let probe = Dpkg;
        self.store.commit_installed(&self.candidate, &probe)?;
        self.store.prune(&[&self.candidate, &self.previous])
    }

    fn rollback(&mut self) -> Result<(), String> {
        let probe = Dpkg;
        let previous_deb = self.store.verify(&self.previous, &probe)?;
        let previous_deb = previous_deb
            .to_str()
            .ok_or("retained previous package path is not valid UTF-8")?
            .to_string();
        let parent = self
            .paths
            .runtime_root
            .parent()
            .ok_or("runtime_root has no parent")?;
        let status = Command::new("tar")
            .arg("xzf")
            .arg(&self.backup)
            .arg("-C")
            .arg(parent)
            .status()
            .map_err(|error| format!("tar restore: {error}"))?;
        if !status.success() {
            return Err("runtime restore (tar) failed".to_string());
        }
        self.install(&previous_deb)?;
        self.store.commit_installed(&self.previous, &probe)?;
        self.store.prune(&[&self.previous, &self.candidate])?;
        Ok(())
    }

    fn finalize(
        &mut self,
        request: &UpgradeFinalizeRequest,
    ) -> Result<UpgradeFinalizeReport, String> {
        let binary = final_version_binary();
        let mut child = Command::new(&binary)
            .args(["upgrade", "--finalize"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("spawn final-version binary {}: {error}", binary.display()))?;
        let mut request = request.clone();
        let held = crate::runtime::held();
        request.soul = held
            .handover_soul
            .clone()
            .unwrap_or_else(|| santi_core::DEFAULT_SOUL_ID.to_string());
        request.configured_strand_id = held.handover_strand.clone();
        let payload = serde_json::to_vec(&request).map_err(|error| error.to_string())?;
        child
            .stdin
            .take()
            .ok_or("final-version binary stdin unavailable")?
            .write_all(&payload)
            .map_err(|error| format!("write finalization request: {error}"))?;
        let output = child
            .wait_with_output()
            .map_err(|error| format!("wait for final-version binary: {error}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!(
                "final-version binary exited with {}: {}",
                output.status,
                stderr.trim()
            ));
        }
        serde_json::from_slice(&output.stdout)
            .map_err(|error| format!("decode final-version report: {error}"))
    }

    fn start(&mut self) -> Result<(), String> {
        self.systemctl("start")
    }
}
