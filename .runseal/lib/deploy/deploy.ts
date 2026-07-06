//! Deploy a santi beta to its live host: fetch the `.deb` on the box and run
//! `santi upgrade` (the self-op orchestration — graceful-stop → snapshot → dpkg
//! → probe → seed → restart), then verify (schema at the new version + the
//! soul-memory md5 unchanged across any DB wipe + health). Does NOT cut a
//! release — that stays `runseal :release` / the release-beta workflow. This is
//! the high-frequency hot op santi owns; the box + edge are consumed as running
//! services, so it carries no dependency on the infra repo.

import { run } from "@/lib/std/cmd.ts";
import { repoRoot } from "@/lib/std/repo.ts";

const HOST = "hk-03.zxiyun";
const SSH_CONFIG = ".local/ssh/config";
const SANTI_HOME = "/home/santi/.santi";
const MEMORY = `${SANTI_HOME}/runtime/souls/soul_default/memory/MEMORY.md`;
const HEALTH = "http://127.0.0.1:43307/api/v1/health";

export async function deploy(argv: string[]): Promise<number> {
  if (argv.includes("-h") || argv.includes("--help")) {
    console.log("Usage: runseal :deploy [<version>]");
    console.log("");
    console.log("Deploy the latest beta (or the given <version>, e.g. v0.1.0-beta.17) to santi's");
    console.log(
      "live host via `santi upgrade`, then verify schema + soul-memory continuity + health.",
    );
    console.log("Cut a release first with `runseal :release` / the release-beta workflow.");
    return 0;
  }
  const version = (argv[0] ?? "").trim();
  const metaPath = version === "" ? "latest" : `versions/${version}`;

  // The on-box orchestration. ssh connects as root; the santi user (passwordless
  // sudo) owns `santi upgrade`. The launcher returns fast; we poll the detached
  // oneshot to completion (graceful-stop can wait out an in-flight turn ≤600s).
  const remote = [
    "set -e",
    `META="https://releases.santi.perish.uk/beta/${metaPath}/metadata.json"`,
    `DEB_URL=$(curl -fsSL "$META" | grep -A5 '"debX64"' | grep '"url"' | sed 's/.*"url": *"//;s/".*//')`,
    '[ -n "$DEB_URL" ] || { echo "no .deb url in $META"; exit 1; }',
    `VER=$(basename "$(dirname "$DEB_URL")")`,
    'echo ">> deploying $VER"',
    'echo ">> $DEB_URL"',
    'curl -fsSL "$DEB_URL" -o /tmp/santi-deploy.deb',
    `BEFORE=$(md5sum ${MEMORY} 2>/dev/null | awk '{print $1}')`,
    `sudo -u santi env SANTI_HOME=${SANTI_HOME} /usr/bin/santi upgrade /tmp/santi-deploy.deb`,
    'for i in $(seq 1 210); do [ "$(systemctl is-active santi-upgrade.service 2>/dev/null)" != activating ] && break; sleep 3; done',
    'echo ">> post-upgrade"',
    'echo "service:   $(systemctl is-active santi.service)"',
    `echo "installed: $(dpkg -l | grep '^ii  santi ' | awk '{print $3}')"`,
    `sudo -u santi env SANTI_HOME=${SANTI_HOME} /usr/bin/santi doctor | grep -E 'schema_version|"ok"' || true`,
    `AFTER=$(md5sum ${MEMORY} 2>/dev/null | awk '{print $1}')`,
    'if [ "$BEFORE" = "$AFTER" ]; then echo "soul-memory md5: UNCHANGED ($AFTER)"; else echo "soul-memory md5: CHANGED before=$BEFORE after=$AFTER"; fi',
    `echo "health: $(curl -fsS ${HEALTH})"`,
  ].join("\n");

  return await run("ssh", ["-F", SSH_CONFIG, HOST, remote], { cwd: repoRoot() });
}
