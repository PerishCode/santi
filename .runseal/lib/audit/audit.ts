//! `runseal :audit [session] [--turn id] [--failed] [-n N] [--full] [-f]`
//!
//! Read-only tool-activity view projected by the local operator binary from the
//! Keel estate. It stays off the HTTP surface while refusing knowledge of
//! Keel's physical schema.

import { capture, sleep } from "@/lib/std/cmd.ts";
import { exists, join } from "@/lib/std/fs.ts";
import { repoRoot } from "@/lib/std/repo.ts";

const DEFAULT_LIMIT = 30;
const FOLLOW_INTERVAL_MS = 2_000;
const HEAD_LINES = 3;
const LINE_MAX = 160;
const ID_RE = /^[A-Za-z0-9_]+$/;
const INDENT = "            ";

interface Options {
  session?: string;
  turn?: string;
  failed: boolean;
  limit: number;
  full: boolean;
  follow: boolean;
}

interface Row {
  created_at: string;
  status: string;
  strand_id: string;
  turn_id: string;
  tool_name: string;
  arguments: unknown;
  output: unknown | null;
  error_text: string | null;
}

interface ShellOutput {
  exit_code?: number;
  stdout?: string;
  stderr?: string;
}

export async function audit(argv: string[]): Promise<number> {
  if (argv.includes("-h") || argv.includes("--help")) {
    usage();
    return 0;
  }
  const opts = parseArgs(argv);
  if (opts instanceof Error) return fail(opts.message);

  const repo = repoRoot();
  const binary = join(repo, "target/debug/santi-api");
  if (!exists(binary)) {
    return fail(`operator binary is missing at ${binary} — run runseal :dev start first`);
  }
  if (opts.session) {
    if (!ID_RE.test(opts.session)) return fail(`invalid session id: ${opts.session}`);
  }
  if (opts.turn) {
    if (!ID_RE.test(opts.turn)) return fail(`invalid turn id: ${opts.turn}`);
  }

  try {
    const recent = await query(binary, opts);
    for (const row of recent) console.log(renderRow(row, opts.full));

    if (!opts.follow) return 0;

    let last = recent.length > 0 ? recent[recent.length - 1].created_at : "";
    while (opts.follow) {
      await sleep(FOLLOW_INTERVAL_MS);
      const fresh = await query(binary, opts, last === "" ? undefined : last);
      for (const row of fresh) {
        console.log(renderRow(row, opts.full));
        last = row.created_at;
      }
    }
    return 0;
  } catch (error) {
    return fail(error instanceof Error ? error.message : String(error));
  }
}

async function query(binary: string, opts: Options, after?: string): Promise<Row[]> {
  const args: string[] = [];
  if (opts.session) args.push("--strand", opts.session);
  args.push("audit", "--limit", String(opts.limit));
  if (opts.turn) args.push("--turn", opts.turn);
  if (opts.failed) args.push("--failed");
  if (after) args.push("--after", after);
  const result = await capture(binary, args);
  if (result.code !== 0) {
    throw new Error(`santi-api audit failed: ${(result.stderr || result.stdout).trim()}`);
  }
  const text = result.stdout.trim();
  return text === "" ? [] : (JSON.parse(text) as Row[]);
}

function renderRow(row: Row, full: boolean): string {
  const head = `${hms(row.created_at)}  ${shortId(row.turn_id)}  ${pad(row.status, 9)}  ` +
    `${row.tool_name}$ ${truncate(commandOf(row), LINE_MAX)}`;
  const body = resultLines(row, full);
  return body.length === 0 ? head : [head, ...body].join("\n");
}

function resultLines(row: Row, full: boolean): string[] {
  if (row.error_text) return block(`✗ ${row.error_text}`, full);

  if (row.output === null) return [];
  const parsed = typeof row.output === "object" ? row.output as ShellOutput : null;
  if (parsed && (parsed.stdout !== undefined || parsed.stderr !== undefined)) {
    const exit = parsed.exit_code ?? 0;
    const marker = exit === 0 ? "→ " : `✗ exit ${exit}: `;
    const text = (parsed.stdout ?? "").trim() || (parsed.stderr ?? "").trim();
    return text === "" ? [INDENT + marker.trimEnd()] : block(marker + text, full);
  }
  return block(`→ ${JSON.stringify(row.output)}`, full);
}

function block(text: string, full: boolean): string[] {
  const lines = text.split("\n");
  const shown = full ? lines : lines.slice(0, HEAD_LINES);
  const rendered = shown.map((line, index) =>
    INDENT + (index === 0 ? truncate(line, LINE_MAX) : "  " + truncate(line, LINE_MAX))
  );
  if (!full && lines.length > HEAD_LINES) rendered.push(`${INDENT}  …`);
  return rendered;
}

function commandOf(row: Row): string {
  if (typeof row.arguments === "object" && row.arguments !== null) {
    const args = row.arguments as { command?: unknown };
    if (typeof args.command === "string") return firstLine(args.command);
  }
  return firstLine(
    typeof row.arguments === "string" ? row.arguments : JSON.stringify(row.arguments),
  );
}

function parseArgs(argv: string[]): Options | Error {
  const opts: Options = { failed: false, limit: DEFAULT_LIMIT, full: false, follow: false };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    switch (arg) {
      case "--failed":
        opts.failed = true;
        break;
      case "--full":
        opts.full = true;
        break;
      case "-f":
      case "--follow":
        opts.follow = true;
        break;
      case "--turn":
        opts.turn = argv[++i];
        if (opts.turn === undefined) return new Error("--turn expects an id");
        break;
      case "-n": {
        const value = Number(argv[++i]);
        if (!Number.isInteger(value) || value <= 0) {
          return new Error("-n expects a positive integer");
        }
        opts.limit = value;
        break;
      }
      default:
        if (arg.startsWith("-")) return new Error(`unknown flag: ${arg}`);
        if (opts.session !== undefined) return new Error(`unexpected argument: ${arg}`);
        opts.session = arg;
    }
  }
  return opts;
}

function hms(iso: string): string {
  const match = iso.match(/T(\d{2}:\d{2}:\d{2})/);
  return match ? match[1] : iso;
}

function shortId(id: string): string {
  const match = id.match(/^([a-z]+_)(.+)$/);
  if (match) return `${match[1]}${match[2].slice(0, 6)}…`;
  return id.length > 12 ? `${id.slice(0, 12)}…` : id;
}

function pad(text: string, width: number): string {
  return text.length >= width ? text : text + " ".repeat(width - text.length);
}

function firstLine(text: string): string {
  const newline = text.indexOf("\n");
  return newline < 0 ? text : `${text.slice(0, newline)} …`;
}

function truncate(text: string, max: number): string {
  return text.length > max ? `${text.slice(0, max - 1)}…` : text;
}

function usage(): void {
  console.log("Usage: runseal :audit [session] [--turn <id>] [--failed] [-n N] [--full] [-f]");
  console.log("");
  console.log("Read-only tool-activity view projected from the local Keel estate.");
  console.log("  session    scope to one strand id (legacy positional spelling)");
  console.log("  --turn id  scope to one turn id");
  console.log("  --failed   only tool errors or failed turns");
  console.log("  -n N       show the last N calls (default 30)");
  console.log("  --full     do not truncate command/output");
  console.log("  -f         follow: poll for new activity");
}

function fail(message: string): number {
  console.error(`:audit: ${message}`);
  return 1;
}
