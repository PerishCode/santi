//! `runseal :audit [session] [--turn id] [--failed] [-n N] [--full] [-f]`
//!
//! Read-only tool-activity view, aggregated straight from the runtime's SQLite
//! store — no runtime code, no API endpoint. It joins tool_calls → turns →
//! tool_results so you can see, for getting work done: when, which turn (and its
//! status), what command ran, and what came back.

import { capture, sleep } from "@/lib/std/cmd.ts";
import { exists, join } from "@/lib/std/fs.ts";
import { repoRoot } from "@/lib/std/repo.ts";

const DEFAULT_DB = ".tmp/santi.sqlite";
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
  turn_id: string;
  tool_name: string;
  arguments: string;
  output: string | null;
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
  const configured = dbPath(repo);
  const db = configured.startsWith("/") ? configured : join(repo, configured);
  if (!exists(db)) {
    return fail(`no database at ${db} — start the server first (runseal :dev start)`);
  }

  const where: string[] = [];
  if (opts.session) {
    if (!ID_RE.test(opts.session)) return fail(`invalid session id: ${opts.session}`);
    where.push(`ss.session_id = '${opts.session}'`);
  }
  if (opts.turn) {
    if (!ID_RE.test(opts.turn)) return fail(`invalid turn id: ${opts.turn}`);
    where.push(`t.id = '${opts.turn}'`);
  }
  if (opts.failed) {
    where.push(
      "(tr.error_text IS NOT NULL OR t.status = 'failed' OR " +
        "IFNULL(json_extract(tr.output, '$.exit_code'), 0) <> 0)",
    );
  }

  try {
    const recent = await query(db, sql(where, `ORDER BY tc.created_at DESC LIMIT ${opts.limit}`));
    recent.reverse();
    for (const row of recent) console.log(renderRow(row, opts.full));

    if (!opts.follow) return 0;

    let last = recent.length > 0 ? recent[recent.length - 1].created_at : "";
    while (opts.follow) {
      await sleep(FOLLOW_INTERVAL_MS);
      const conditions = last === "" ? where : [...where, `tc.created_at > '${last}'`];
      const fresh = await query(db, sql(conditions, "ORDER BY tc.created_at ASC"));
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

function sql(where: string[], tail: string): string {
  const clause = where.length > 0 ? `WHERE ${where.join(" AND ")}` : "";
  return `
SELECT tc.created_at AS created_at, t.status AS status, tc.turn_id AS turn_id,
       tc.tool_name AS tool_name, tc.arguments AS arguments,
       tr.output AS output, tr.error_text AS error_text
FROM tool_calls tc
JOIN turns t ON t.id = tc.turn_id
LEFT JOIN tool_results tr ON tr.tool_call_id = tc.id
JOIN soul_sessions ss ON ss.id = t.soul_session_id
${clause}
${tail};`;
}

async function query(db: string, statement: string): Promise<Row[]> {
  const result = await capture("sqlite3", ["-readonly", "-json", db, statement]);
  if (result.code !== 0) {
    throw new Error(`sqlite3 failed: ${(result.stderr || result.stdout).trim()}`);
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
  let parsed: ShellOutput | null = null;
  try {
    parsed = JSON.parse(row.output) as ShellOutput;
  } catch {
    parsed = null;
  }
  if (parsed && (parsed.stdout !== undefined || parsed.stderr !== undefined)) {
    const exit = parsed.exit_code ?? 0;
    const marker = exit === 0 ? "→ " : `✗ exit ${exit}: `;
    const text = (parsed.stdout ?? "").trim() || (parsed.stderr ?? "").trim();
    return text === "" ? [INDENT + marker.trimEnd()] : block(marker + text, full);
  }
  return block(`→ ${row.output}`, full);
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
  try {
    const args = JSON.parse(row.arguments) as { command?: string };
    if (typeof args.command === "string") return firstLine(args.command);
  } catch {
    // not JSON / not a shell call; fall back to the raw arguments
  }
  return firstLine(row.arguments);
}

function dbPath(repo: string): string {
  const fromEnv = Deno.env.get("SANTI_DB");
  if (fromEnv) return fromEnv;
  try {
    const match = Deno.readTextFileSync(join(repo, ".env")).match(/^\s*SANTI_DB\s*=\s*(\S+)\s*$/m);
    if (match) return match[1];
  } catch {
    // no .env; use the default
  }
  return DEFAULT_DB;
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
  console.log("Read-only tool-activity view aggregated from the runtime SQLite store.");
  console.log("  session    scope to one session id");
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
