//! Publish-independent live deployment: resolve a beta, run the streamed
//! host-side transaction, then arm its post-deploy recovery capsule.

import { runDeployRemote } from "@/lib/deploy/remote.ts";
import { armRecovery, guardDeploy } from "@/lib/recovery/recovery.ts";

const LATEST = "https://releases.santi.perish.uk/beta/latest/metadata.json";
const VERSION = /^v[0-9]+\.[0-9]+\.[0-9]+-beta\.[1-9][0-9]*$/;

export async function deploy(argv: string[]): Promise<number> {
  if (argv.includes("-h") || argv.includes("--help")) {
    console.log("Usage: runseal :deploy [<version>]");
    console.log("");
    console.log("Deploy the latest beta (or an explicit vX.Y.Z-beta.N) to the live host.");
    console.log("The host transaction snapshots, installs, verifies, and rolls back on failure;");
    console.log("a successful candidate is then protected by an armed recovery capsule.");
    return 0;
  }
  if (argv.length > 1) {
    console.error(":deploy: expected at most one version");
    return 2;
  }

  try {
    const version = argv.length === 1 ? validate(argv[0]) : await latest();
    const recoveryGate = await guardDeploy();
    if (recoveryGate !== 0) return recoveryGate;
    const deployed = await runDeployRemote(version);
    if (deployed !== 0) return deployed;
    return await armRecovery();
  } catch (error) {
    console.error(`:deploy: ${error instanceof Error ? error.message : String(error)}`);
    return 1;
  }
}

function validate(value: string): string {
  const version = value.trim();
  if (!VERSION.test(version)) throw new Error(`invalid beta version: ${JSON.stringify(value)}`);
  return version;
}

async function latest(): Promise<string> {
  const response = await fetch(LATEST, {
    headers: { "Cache-Control": "no-cache", "User-Agent": "santi-deploy/1.0" },
  });
  if (!response.ok) throw new Error(`latest beta metadata returned HTTP ${response.status}`);
  const metadata = await response.json() as { releaseVersion?: unknown };
  if (typeof metadata.releaseVersion !== "string") {
    throw new Error("latest beta metadata has no releaseVersion");
  }
  return validate(metadata.releaseVersion);
}
