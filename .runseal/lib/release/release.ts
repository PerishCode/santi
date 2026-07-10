//! `runseal :release <step>` — one step of the release pipeline, driven by the
//! CI workflow (the YAML owns the job DAG + build matrix; each step's logic
//! lives here). Inputs come from the environment, mirroring the workflow.
//!
//! Steps:
//!   access-check     write-probe R2 to confirm credentials (metadata job)
//!   metadata         resolve the next version → GITHUB_OUTPUT
//!   package          build santi for $TARGET and archive into dist/<version>/
//!   publish          checksums + accept + verify members + upload + metadata
//!   verify-publish   re-fetch published metadata, validate, HEAD every URL
//!   smoke            install from the public URL, run --help + service health

import { repoRoot } from "@/lib/std/repo.ts";
import { fail, required } from "@/lib/release/env.ts";
import { type Channel, resolveVersion } from "@/lib/release/meta.ts";
import { accessCheck } from "@/lib/release/r2.ts";
import { accept, checksums, pkg, verifyMembers } from "@/lib/release/artifacts.ts";
import { publish, verifyPublish } from "@/lib/release/publish.ts";
import { smoke } from "@/lib/release/smoke.ts";

const STEPS = [
  "access-check",
  "metadata",
  "package",
  "publish",
  "verify-publish",
  "smoke",
] as const;

export async function release(argv: string[]): Promise<number> {
  if (argv.includes("-h") || argv.includes("--help")) {
    usage();
    return 0;
  }
  const step = argv[0];
  const repo = repoRoot();
  switch (step) {
    case "access-check":
      await accessCheck(channel());
      return 0;
    case "metadata":
      return await resolveVersion(channel(), repo);
    case "package":
      await pkg(repo);
      return 0;
    case "publish":
      await checksums(repo);
      accept(repo);
      await verifyMembers(repo);
      await publish(repo);
      return 0;
    case "verify-publish":
      await verifyPublish();
      return 0;
    case "smoke":
      await smoke(repo);
      return 0;
    default:
      fail(`unknown step: ${step ?? "<none>"} (expected ${STEPS.join("|")})`);
  }
}

function usage(): void {
  console.log("Usage: runseal :release <step>");
  console.log("");
  console.log("Run one internal step of the CI release pipeline.");
  console.log("This wrapper does not dispatch a release workflow by itself.");
  console.log("");
  console.log("Steps:");
  console.log("  access-check    Probe release storage credentials for the selected channel");
  console.log("  metadata        Resolve the next release version and write GITHUB_OUTPUT");
  console.log("  package         Build and archive santi for TARGET");
  console.log("  publish         Build checksums, verify members, and publish artifacts");
  console.log("  verify-publish  Re-fetch metadata and verify every published artifact URL");
  console.log("  smoke           Install from the public channel, probe health, and uninstall");
  console.log("");
  console.log("Environment:");
  console.log("  RELEASE_CHANNEL=beta|stable is required where the selected step uses it.");
  console.log("  Step-specific release variables and credentials are supplied by CI.");
  console.log("");
  console.log("Warning: access-check and publish can write to release storage.");
}

function channel(): Channel {
  const value = required("RELEASE_CHANNEL");
  if (value !== "beta" && value !== "stable") {
    fail(`RELEASE_CHANNEL must be beta|stable, got ${value}`);
  }
  return value;
}
