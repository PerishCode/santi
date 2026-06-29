//! `runseal :land <title> [--body <text>]`
//!
//! Consolidates the squash-merge hot path into one blocking call so the
//! operator (human or agent) does not burn round-trips polling CI:
//!
//!   clean non-main branch → push → reuse/create PR → wait for required checks
//!   (polled inside this wrapper) → squash merge --delete-branch → sync main.
//!
//! Fail-fast everywhere: missing title, on main, dirty tree, no commits ahead,
//! failing/timed-out checks — all stop with a message and leave the PR open. It
//! never commits, branches, or rebases on your behalf.

import { capture, run, sleep } from "@/lib/std/cmd.ts";
import { repoRoot } from "@/lib/std/repo.ts";

const BASE = "main";
// ~2x the observed ~110s `smoke (ubuntu-latest)` duration on PRs.
const CHECK_TIMEOUT_MS = 220_000;
const POLL_PENDING_MS = 10_000;
const POLL_REGISTER_MS = 5_000;

export async function land(argv: string[]): Promise<number> {
  const { title, body } = parseArgs(argv);
  if (!title) {
    return fail("title is required — usage: runseal :land <title> [--body <text>]");
  }
  const repo = repoRoot();

  // 1. Preconditions — fail fast, never mutate the working state.
  const branch = (await capture("git", ["rev-parse", "--abbrev-ref", "HEAD"], { cwd: repo }))
    .stdout.trim();
  if (branch === "" || branch === "HEAD") return fail("detached HEAD — checkout a branch first");
  if (branch === BASE) return fail(`on ${BASE} — :land lands a non-${BASE} branch`);
  if ((await capture("git", ["status", "--porcelain"], { cwd: repo })).stdout.trim() !== "") {
    return fail("working tree not clean — commit or stash first");
  }
  const ahead = (await capture("git", ["rev-list", "--count", `${BASE}..HEAD`], { cwd: repo }))
    .stdout.trim();
  if (Number(ahead) === 0) return fail(`no commits on ${branch} ahead of ${BASE}`);

  // 2. Push.
  console.log(`pushing ${branch} ...`);
  if ((await run("git", ["push", "-u", "origin", branch], { cwd: repo })) !== 0) {
    return fail("git push failed");
  }

  // 3. PR — reuse an open one for this branch, else create.
  let pr = await openPr(branch, repo);
  if (pr === null) {
    console.log("creating PR ...");
    const created = await capture("gh", [
      "pr",
      "create",
      "--base",
      BASE,
      "--head",
      branch,
      "--title",
      title,
      "--body",
      body ?? "",
    ], { cwd: repo });
    if (created.code !== 0) return fail(`gh pr create failed: ${oneLine(created.stderr)}`);
    pr = await openPr(branch, repo);
    if (pr === null) return fail("PR created but could not be resolved");
  } else {
    console.log(`reusing PR #${pr}`);
  }

  // 4. Wait for required checks — polled here, quietly, with a bounded timeout.
  console.log(`waiting for required checks (timeout ${CHECK_TIMEOUT_MS / 1000}s) ...`);
  const outcome = await waitChecks(pr, repo);
  if (outcome !== "pass") {
    return fail(`required checks ${outcome} — PR #${pr} left open`);
  }

  // 5. Squash merge.
  console.log(`squash-merging PR #${pr} ...`);
  const merged = await capture(
    "gh",
    ["pr", "merge", String(pr), "--squash", "--delete-branch"],
    { cwd: repo },
  );
  if (merged.code !== 0) return fail(`gh pr merge failed: ${oneLine(merged.stderr)}`);

  // 6. Sync base.
  if ((await run("git", ["checkout", BASE], { cwd: repo })) !== 0) {
    return fail(`merged PR #${pr}, but could not checkout ${BASE} — sync manually`);
  }
  if ((await run("git", ["pull", "--ff-only"], { cwd: repo })) !== 0) {
    return fail(`merged PR #${pr}, but git pull --ff-only failed — sync manually`);
  }
  const head = (await capture("git", ["rev-parse", "--short", "HEAD"], { cwd: repo })).stdout
    .trim();
  console.log(`landed PR #${pr} → ${BASE} ${head}`);
  return 0;
}

async function openPr(branch: string, repo: string): Promise<number | null> {
  const result = await capture("gh", [
    "pr",
    "list",
    "--head",
    branch,
    "--state",
    "open",
    "--json",
    "number",
    "-q",
    ".[0].number",
  ], { cwd: repo });
  const value = Number(result.stdout.trim());
  return Number.isInteger(value) && value > 0 ? value : null;
}

/**
 * Poll the PR's required checks until they all pass, one fails, or the timeout
 * elapses. Parses `--json bucket` from stdout and ignores exit codes so pending
 * states are handled uniformly; an empty set means checks have not registered
 * yet (just after creation).
 */
async function waitChecks(pr: number, repo: string): Promise<"pass" | "fail" | "timeout"> {
  const deadline = Date.now() + CHECK_TIMEOUT_MS;
  while (Date.now() < deadline) {
    const result = await capture(
      "gh",
      ["pr", "checks", String(pr), "--required", "--json", "bucket"],
      { cwd: repo },
    );
    let buckets: string[] = [];
    try {
      buckets = (JSON.parse(result.stdout) as { bucket: string }[]).map((check) => check.bucket);
    } catch {
      buckets = [];
    }
    if (buckets.length === 0) {
      await sleep(POLL_REGISTER_MS); // not registered yet
      continue;
    }
    if (buckets.some((bucket) => bucket === "fail" || bucket === "cancel")) return "fail";
    if (buckets.every((bucket) => bucket === "pass" || bucket === "skipping")) return "pass";
    await sleep(POLL_PENDING_MS); // some still pending
  }
  return "timeout";
}

function parseArgs(argv: string[]): { title?: string; body?: string } {
  let title: string | undefined;
  let body: string | undefined;
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--body") {
      body = argv[++i];
    } else if (!arg.startsWith("--") && title === undefined) {
      title = arg;
    }
  }
  return { title, body };
}

function oneLine(text: string): string {
  return text.trim().split("\n").join(" ");
}

function fail(message: string): number {
  console.error(`:land: ${message}`);
  return 1;
}
