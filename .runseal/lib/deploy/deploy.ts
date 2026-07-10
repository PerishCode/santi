//! Deploy a santi beta to its live host: fetch the `.deb` on the box and run
//! `santi upgrade` (the self-op orchestration — graceful-stop → snapshot → dpkg
//! → probe → seed → restart), then verify (schema at the new version + the
//! soul-memory md5 unchanged across any DB wipe + explicit readiness). Does NOT cut a
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
      "live host via `santi upgrade`, preserving the installed beta for truthful rollback,",
    );
    console.log("then verify schema + memory + ready/degraded state.");
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
    'ROLLBACK_ENV_SET=""',
    "cleanup() {",
    '  if [ "$ROLLBACK_ENV_SET" = 1 ]; then systemctl unset-environment SANTI_PREVIOUS_DEB || true; fi',
    "}",
    "trap cleanup EXIT",
    "INITIAL_UPGRADE_STATE=$(systemctl is-active santi-upgrade.service 2>/dev/null || true)",
    '[ "$INITIAL_UPGRADE_STATE" != activating ] || { echo "upgrade already active"; exit 1; }',
    "CURRENT_PACKAGE_VERSION=$(dpkg-query -W -f='${Version}' santi 2>/dev/null || true)",
    '[ -n "$CURRENT_PACKAGE_VERSION" ] || { echo "installed santi package version is unavailable"; exit 1; }',
    `META="https://releases.santi.perish.uk/beta/${metaPath}/metadata.json"`,
    `DEB_URL=$(curl -fsSL "$META" | grep -A5 '"debX64"' | grep '"url"' | sed 's/.*"url": *"//;s/".*//')`,
    '[ -n "$DEB_URL" ] || { echo "no .deb url in $META"; exit 1; }',
    `VER=$(basename "$(dirname "$DEB_URL")")`,
    'EXPECTED_PACKAGE_VERSION="${VER#v}"',
    'echo ">> deploying $VER"',
    'echo ">> $DEB_URL"',
    'curl -fsSL "$DEB_URL" -o /tmp/santi-deploy.deb',
    'PREVIOUS_META="https://releases.santi.perish.uk/beta/versions/v${CURRENT_PACKAGE_VERSION}/metadata.json"',
    `PREVIOUS_DEB_URL=$(curl -fsSL "$PREVIOUS_META" | grep -A5 '"debX64"' | grep '"url"' | sed 's/.*"url": *"//;s/".*//')`,
    '[ -n "$PREVIOUS_DEB_URL" ] || { echo "no rollback .deb url in $PREVIOUS_META"; exit 1; }',
    'curl -fsSL "$PREVIOUS_DEB_URL" -o /tmp/santi-previous.deb',
    "PREVIOUS_DEB_PACKAGE=$(dpkg-deb -f /tmp/santi-previous.deb Package)",
    "PREVIOUS_DEB_VERSION=$(dpkg-deb -f /tmp/santi-previous.deb Version)",
    '[ "$PREVIOUS_DEB_PACKAGE" = santi ] || { echo "rollback package mismatch: $PREVIOUS_DEB_PACKAGE"; exit 1; }',
    '[ "$PREVIOUS_DEB_VERSION" = "$CURRENT_PACKAGE_VERSION" ] || { echo "rollback version mismatch: expected=$CURRENT_PACKAGE_VERSION actual=$PREVIOUS_DEB_VERSION"; exit 1; }',
    "chmod 0644 /tmp/santi-previous.deb",
    'echo ">> rollback package $PREVIOUS_DEB_VERSION"',
    'ROLLBACK_ENV_SET="1"',
    "systemctl set-environment SANTI_PREVIOUS_DEB=/tmp/santi-previous.deb",
    `BEFORE=$(md5sum ${MEMORY} 2>/dev/null | awk '{print $1}')`,
    `sudo -u santi env SANTI_HOME=${SANTI_HOME} SANTI_PREVIOUS_DEB=/tmp/santi-previous.deb /usr/bin/santi upgrade /tmp/santi-deploy.deb`,
    "sleep 1",
    'UPGRADE_STATE=""',
    'INSTALLED_VERSION=""',
    "for i in $(seq 1 210); do",
    "  UPGRADE_STATE=$(systemctl is-active santi-upgrade.service 2>/dev/null || true)",
    "  INSTALLED_VERSION=$(dpkg-query -W -f='${Version}' santi 2>/dev/null || true)",
    '  if [ "$UPGRADE_STATE" != activating ] && [ "$INSTALLED_VERSION" = "$EXPECTED_PACKAGE_VERSION" ]; then break; fi',
    "  sleep 3",
    "done",
    '[ "$UPGRADE_STATE" != activating ] || { echo "upgrade timed out"; exit 1; }',
    '[ "$INSTALLED_VERSION" = "$EXPECTED_PACKAGE_VERSION" ] || { echo "installed version mismatch: expected=$EXPECTED_PACKAGE_VERSION actual=$INSTALLED_VERSION"; exit 1; }',
    "UPGRADE_RESULT=$(systemctl show santi-upgrade.service -p Result --value)",
    '[ "$UPGRADE_RESULT" = success ] || { echo "upgrade unit failed: $UPGRADE_RESULT"; exit 1; }',
    'echo ">> post-upgrade"',
    "SERVICE_STATE=$(systemctl is-active santi.service)",
    'echo "service:   $SERVICE_STATE"',
    'echo "installed: $INSTALLED_VERSION"',
    "set -a",
    ". /etc/santi/santi.env",
    "set +a",
    `env SANTI_HOME=${SANTI_HOME} /usr/bin/santi doctor`,
    `AFTER=$(md5sum ${MEMORY} 2>/dev/null | awk '{print $1}')`,
    '[ -n "$BEFORE" ] && [ "$BEFORE" = "$AFTER" ] || { echo "soul-memory md5: CHANGED before=$BEFORE after=$AFTER"; exit 1; }',
    'echo "soul-memory md5: UNCHANGED ($AFTER)"',
    `HEALTH_CODE=$(curl -sS -o /tmp/santi-health.json -w '%{http_code}' ${HEALTH})`,
    "HEALTH_JSON=$(cat /tmp/santi-health.json)",
    'echo "health: $HEALTH_JSON"',
    'if [ "$HEALTH_CODE" = 200 ]; then',
    '  echo "deploy readiness: READY"',
    'elif [ "$HEALTH_CODE" = 503 ]; then',
    '  echo "deploy readiness: DEGRADED - package is live; inspect incidents and run santi strand drive"',
    "else",
    '  echo "unexpected health response: HTTP $HEALTH_CODE $HEALTH_JSON"',
    "  exit 1",
    "fi",
  ].join("\n");

  return await run("ssh", ["-F", SSH_CONFIG, HOST, remote], { cwd: repoRoot() });
}
