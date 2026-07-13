//! Forgejo landing flow: push one clean topic branch, create or reuse its PR,
//! wait for the exact head guard, squash-merge it, and synchronize local main.

import { capture, run } from "@/lib/std/cmd.ts";
import { repoRoot } from "@/lib/std/repo.ts";

interface Options {
  title: string;
  body: string;
  base: string;
  repo: string;
  deleteBranch: boolean;
}

export async function land(argv: string[]): Promise<number> {
  if (argv.includes("-h") || argv.includes("--help")) {
    usage();
    return 0;
  }

  try {
    const options = parse(argv);
    if (options.title === "") throw new Error("title is required");
    const root = repoRoot();
    const branch = await text("git", ["branch", "--show-current"], root);
    if (branch === "") throw new Error("detached HEAD — checkout a topic branch first");
    if (branch === options.base || branch === "main" || branch === "master") {
      throw new Error(`must run on a topic branch, not ${branch}`);
    }
    if ((await text("git", ["status", "--short"], root)) !== "") {
      throw new Error("working tree must be clean; commit or stash changes first");
    }

    await command("git", ["fetch", "origin", options.base], root);
    const remote = `origin/${options.base}`;
    if (!(await ok("git", ["merge-base", "--is-ancestor", remote, "HEAD"], root))) {
      throw new Error(`current branch must contain latest ${remote}; rebase first`);
    }
    const ahead = Number(await text("git", ["rev-list", "--count", `${remote}..HEAD`], root));
    if (!Number.isFinite(ahead) || ahead <= 0) {
      throw new Error(`current branch has no commits ahead of ${remote}`);
    }

    await command("git", ["push", "-u", "origin", branch], root);
    const target = options.repo || await targetRepo(root);
    let pull = await forgejoJson([
      "pr",
      "find",
      "--repo",
      target,
      "--head",
      branch,
      "--base",
      options.base,
    ], root);
    if (pull === null) {
      const args = [
        "pr",
        "create",
        "--repo",
        target,
        "--head",
        branch,
        "--base",
        options.base,
        "--title",
        options.title,
      ];
      if (options.body !== "") args.push("--body", options.body);
      pull = await forgejoJson(args, root);
    }

    const number = integerField(pull, "number");
    const url = stringField(pull, "html_url");
    console.log(url);
    const guarded = await forgejoJson([
      "pr",
      "guard",
      "--repo",
      target,
      "--number",
      String(number),
    ], root);
    const sha = stringField(guarded, "commit_sha");
    await forgejo([
      "pr",
      "merge",
      "--repo",
      target,
      "--number",
      String(number),
      "--head",
      sha,
      "--delete-branch",
      String(options.deleteBranch),
    ], root);

    await command("git", ["checkout", options.base], root);
    await command("git", ["pull", "--ff-only", "origin", options.base], root);
    if (options.deleteBranch && await ok("git", ["rev-parse", "--verify", branch], root)) {
      await command("git", ["branch", "-D", branch], root);
    }
    console.log(`landed Forgejo PR #${number} on ${options.base}`);
    return 0;
  } catch (error) {
    return fail(error instanceof Error ? error.message : String(error));
  }
}

function usage(): void {
  console.log("Usage: runseal :land <title> [options]");
  console.log("");
  console.log("Push and squash-merge the current clean topic branch through Forgejo.");
  console.log("");
  console.log("Options:");
  console.log("  --body <text>       pull request body");
  console.log("  --base <branch>     base branch (default: main)");
  console.log("  --repo <owner/name> Forgejo repository (default: derived from origin)");
  console.log("  --no-delete         keep the topic branch after merge");
}

function parse(argv: string[]): Options {
  const values = [...argv];
  let title = "";
  let body = "";
  let base = "main";
  let repo = "";
  let deleteBranch = true;
  for (let index = 0; index < values.length; index++) {
    const arg = values[index];
    if (arg === "--body" || arg === "--base" || arg === "--repo") {
      const value = values[++index];
      if (value === undefined) throw new Error(`${arg} requires a value`);
      if (arg === "--body") body = value;
      if (arg === "--base") base = value;
      if (arg === "--repo") repo = value;
    } else if (arg === "--no-delete") {
      deleteBranch = false;
    } else if (arg.startsWith("--")) {
      throw new Error(`unknown option: ${arg}`);
    } else if (title === "") {
      title = arg;
    } else {
      throw new Error(`unexpected argument: ${arg}`);
    }
  }
  return { title, body, base, repo, deleteBranch };
}

async function targetRepo(root: string): Promise<string> {
  const origin = (await text("git", ["remote", "get-url", "origin"], root)).replace(/\.git$/, "");
  const found = origin.match(/[:/]([^/:]+)\/([^/]+)$/);
  if (found === null) throw new Error(`cannot derive Forgejo owner/name from origin: ${origin}`);
  return `${found[1]}/${found[2]}`;
}

async function forgejo(args: string[], root: string): Promise<string> {
  const result = await capture("runseal", ["@tool", "forgejo", ...args], { cwd: root });
  if (result.code !== 0) throw new Error(`Forgejo ${args[0]} failed: ${oneLine(result.stderr)}`);
  return result.stdout.trim();
}

async function forgejoJson(args: string[], root: string): Promise<unknown> {
  const raw = await forgejo(args, root);
  try {
    return JSON.parse(raw);
  } catch {
    throw new Error(`Forgejo ${args[0]} returned invalid JSON`);
  }
}

async function text(command: string, args: string[], root: string): Promise<string> {
  const result = await capture(command, args, { cwd: root });
  if (result.code !== 0) throw new Error(`${command} ${args[0]} failed: ${oneLine(result.stderr)}`);
  return result.stdout.trim();
}

async function command(command: string, args: string[], root: string): Promise<void> {
  if (await run(command, args, { cwd: root }) !== 0) {
    throw new Error(`${command} ${args[0]} failed`);
  }
}

async function ok(command: string, args: string[], root: string): Promise<boolean> {
  return (await capture(command, args, { cwd: root })).code === 0;
}

function integerField(value: unknown, name: string): number {
  if (typeof value !== "object" || value === null) throw new Error(`missing Forgejo ${name}`);
  const found = (value as Record<string, unknown>)[name];
  if (!Number.isInteger(found) || Number(found) <= 0) throw new Error(`invalid Forgejo ${name}`);
  return Number(found);
}

function stringField(value: unknown, name: string): string {
  if (typeof value !== "object" || value === null) throw new Error(`missing Forgejo ${name}`);
  const found = (value as Record<string, unknown>)[name];
  if (typeof found !== "string" || found === "") throw new Error(`invalid Forgejo ${name}`);
  return found;
}

function oneLine(text: string): string {
  return text.trim().split(/\r?\n/).join(" ");
}

function fail(message: string): number {
  console.error(`:land: ${message}`);
  return 1;
}
