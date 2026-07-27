//! `runseal :guard` — the local validation suite the pre-commit hook runs.
//!
//! Mirrors the CI repo + Rust checks (Negentropy and cargo, all `--locked` like CI)
//! so "green locally" means "green in CI", plus the Deno checks for the
//! `.runseal/` wrappers that CI does not cover. Runnable on demand as well as
//! from the hook.

import { run } from "@/lib/std/cmd.ts";
import { join } from "@/lib/std/fs.ts";
import { repoRoot } from "@/lib/std/repo.ts";
import {
  classifiedScan,
  formatReport,
  lawLines,
  loadBaseline,
  ratchet,
} from "@/lib/word/report.ts";

interface Step {
  title: string;
  command: string;
  args: string[];
}

export async function guard(argv: string[]): Promise<number> {
  if (argv.includes("-h") || argv.includes("--help")) {
    usage();
    return 0;
  }
  if (argv.length > 0) {
    console.error(`:guard: unexpected argument: ${argv[0]}`);
    usage();
    return 2;
  }
  const repo = repoRoot();
  const wrappers = wrapperFiles(repo);
  const config = ".runseal/deno.json";

  console.log("==> negentropy");
  try {
    const { report, raw, notes, faults } = await classifiedScan(repo);
    for (const line of lawLines(raw)) {
      console.log(line);
    }
    for (const note of notes) {
      console.log(note);
    }
    if (faults.length > 0) {
      for (const fault of faults) {
        console.error(fault);
      }
      console.error(`:guard: wire-seat gate failed (${faults.length} fault(s))`);
      return 1;
    }
    console.log(formatReport(report));
    const baseline = await loadBaseline(repo);
    const result = ratchet(report, baseline);
    for (const message of result.messages) {
      console.log(message);
    }
    if (result.code !== 0) {
      console.error(":guard: word-debt gate failed");
      return result.code;
    }
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    console.error(":guard: negentropy / word-debt failed");
    return 1;
  }

  const steps: Step[] = [
    { title: "cargo fmt", command: "cargo", args: ["fmt", "--all", "--check"] },
    {
      title: "cargo clippy",
      command: "cargo",
      args: ["clippy", "--locked", "--workspace", "--all-targets", "--", "-D", "warnings"],
    },
    { title: "cargo test", command: "cargo", args: ["test", "--locked", "--workspace"] },
    {
      title: "recovery capsule",
      command: "bash",
      args: [".runseal/ops/host/30-santi-recovery.test.sh"],
    },
    {
      title: "deploy transaction",
      command: "bash",
      args: [".runseal/ops/host/40-santi-deploy.test.sh"],
    },
    {
      title: "deno fmt",
      command: "deno",
      args: ["fmt", "--config", config, "--check", ".runseal"],
    },
    { title: "deno lint", command: "deno", args: ["lint", "--config", config, ".runseal"] },
    { title: "deno test", command: "deno", args: ["test", "--config", config, ".runseal/lib"] },
    { title: "deno check", command: "deno", args: ["check", "--config", config, ...wrappers] },
  ];

  for (const step of steps) {
    console.log(`==> ${step.title}`);
    const code = await run(step.command, step.args, { cwd: repo });
    if (code !== 0) {
      console.error(`:guard: ${step.title} failed`);
      return code;
    }
  }
  console.log("guard: ok");
  return 0;
}

function usage(): void {
  console.log("Usage: runseal :guard");
  console.log("");
  console.log("Run the local Negentropy, Rust, and runseal validation suite used before landing.");
}

/** Discover wrapper entrypoints so `deno check` covers them (and their libs). */
function wrapperFiles(repo: string): string[] {
  const files: string[] = [];
  for (const entry of Deno.readDirSync(join(repo, ".runseal/wrappers"))) {
    if (entry.isFile && entry.name.endsWith(".ts")) {
      files.push(`.runseal/wrappers/${entry.name}`);
    }
  }
  return files.sort();
}
