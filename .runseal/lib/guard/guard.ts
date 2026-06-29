//! `runseal :guard` — the local validation suite the pre-commit hook runs.
//!
//! Mirrors the CI `smoke (ubuntu-latest)` job (cargo fmt/clippy/test, all
//! `--locked` like CI) so "green locally" means "green in CI", plus the Deno
//! checks for the `.runseal/` wrappers that CI does not cover. Runnable on
//! demand as well as from the hook.

import { run } from "@/lib/std/cmd.ts";
import { join } from "@/lib/std/fs.ts";
import { repoRoot } from "@/lib/std/repo.ts";

interface Step {
  title: string;
  command: string;
  args: string[];
}

export async function guard(): Promise<number> {
  const repo = repoRoot();
  const wrappers = wrapperFiles(repo);
  const config = ".runseal/deno.json";

  const steps: Step[] = [
    { title: "cargo fmt", command: "cargo", args: ["fmt", "--all", "--check"] },
    {
      title: "cargo clippy",
      command: "cargo",
      args: ["clippy", "--locked", "--workspace", "--all-targets", "--", "-D", "warnings"],
    },
    { title: "cargo test", command: "cargo", args: ["test", "--locked", "--workspace"] },
    {
      title: "deno fmt",
      command: "deno",
      args: ["fmt", "--config", config, "--check", ".runseal"],
    },
    { title: "deno lint", command: "deno", args: ["lint", "--config", config, ".runseal"] },
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
