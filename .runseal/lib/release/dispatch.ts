//! Operator-facing Forgejo release dispatch and optional run watch.

import { capture, run } from "@/lib/std/cmd.ts";
import { repoRoot } from "@/lib/std/repo.ts";

interface Options {
  channel: string;
  repo: string;
  ref: string;
  version: string;
  watch: boolean;
  dryRun: boolean;
}

export async function dispatch(argv: string[]): Promise<number> {
  if (argv.length === 0 || argv.includes("-h") || argv.includes("--help")) {
    usage();
    return 0;
  }

  try {
    const options = parse(argv);
    const workflow = workflowFor(options.channel);
    const root = repoRoot();
    const target = options.repo || await targetRepo(root);
    const args = [
      "@tool",
      "forgejo",
      "workflow",
      "dispatch",
      "--repo",
      target,
      "--workflow",
      workflow,
      "--ref",
      options.ref,
      "--input",
      `version_override=${options.version}`,
    ];
    if (options.dryRun) {
      console.log(`runseal ${args.join(" ")}`);
      return 0;
    }

    const result = await capture("runseal", args, { cwd: root });
    if (result.code !== 0) throw new Error(`dispatch failed: ${oneLine(result.stderr)}`);
    const value = JSON.parse(result.stdout) as Record<string, unknown>;
    const id = Number(value.id);
    if (!Number.isInteger(id) || id <= 0) throw new Error("dispatch returned no run id");
    console.log(`triggered ${workflow} run ${id} for ${options.ref}`);
    if (!options.watch) return 0;
    return await run("runseal", [
      "@tool",
      "forgejo",
      "run",
      "watch",
      "--repo",
      target,
      "--id",
      String(id),
      "--interval",
      "10",
    ], { cwd: root });
  } catch (error) {
    console.error(`:release: ${error instanceof Error ? error.message : String(error)}`);
    return 1;
  }
}

function usage(): void {
  console.log("Usage: runseal :release --channel beta|stable [options]");
  console.log("");
  console.log("Trigger a Forgejo release workflow.");
  console.log("");
  console.log("Options:");
  console.log("  --channel <name>      release channel: beta or stable");
  console.log("  --repo <owner/name>   Forgejo repository (default: derived from origin)");
  console.log("  --ref <ref>           git ref to release (default: main)");
  console.log("  --version <version>   optional release version override");
  console.log("  --watch               wait for the triggered workflow run");
  console.log("  --dry-run             print the dispatch command only");
}

function parse(argv: string[]): Options {
  let channel = "";
  let repo = "";
  let ref = "main";
  let version = "";
  let watch = false;
  let dryRun = false;
  for (let index = 0; index < argv.length; index++) {
    const arg = argv[index];
    const pair = arg.match(/^--(channel|repo|ref|version)=(.*)$/);
    if (pair !== null) {
      ({ channel, repo, ref, version } = assign(pair[1], pair[2], { channel, repo, ref, version }));
    } else if (["--channel", "--repo", "--ref", "--version"].includes(arg)) {
      const value = argv[++index];
      if (value === undefined) throw new Error(`${arg} requires a value`);
      ({ channel, repo, ref, version } = assign(arg.slice(2), value, {
        channel,
        repo,
        ref,
        version,
      }));
    } else if (arg === "--watch") {
      watch = true;
    } else if (arg === "--dry-run") {
      dryRun = true;
    } else {
      throw new Error(`unknown argument: ${arg}`);
    }
  }
  if (channel === "") throw new Error("--channel is required");
  return { channel, repo, ref, version, watch, dryRun };
}

function assign(
  name: string,
  value: string,
  options: Pick<Options, "channel" | "repo" | "ref" | "version">,
): Pick<Options, "channel" | "repo" | "ref" | "version"> {
  return { ...options, [name]: value };
}

function workflowFor(channel: string): string {
  if (channel === "beta") return "release-beta.yml";
  if (channel === "stable") return "release-stable.yml";
  throw new Error(`invalid channel: ${channel}`);
}

async function targetRepo(root: string): Promise<string> {
  const result = await capture("git", ["remote", "get-url", "origin"], { cwd: root });
  if (result.code !== 0) throw new Error(`cannot read origin: ${oneLine(result.stderr)}`);
  const origin = result.stdout.trim().replace(/\.git$/, "");
  const found = origin.match(/[:/]([^/:]+)\/([^/]+)$/);
  if (found === null) throw new Error(`cannot derive Forgejo owner/name from origin: ${origin}`);
  return `${found[1]}/${found[2]}`;
}

function oneLine(text: string): string {
  return text.trim().split(/\r?\n/).join(" ");
}
