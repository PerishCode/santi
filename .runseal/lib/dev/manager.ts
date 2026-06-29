//! A pm2-like, lightweight process manager for the local santi-api dev server,
//! built on Deno-native APIs.
//!
//! Process tree:
//!   runseal :dev start                      (short-lived operator)
//!     └─ deno dev.ts __run --santi-stamp=…   (stamped launcher, persistent)
//!          └─ target/debug/santi-api serve   (the actual server)
//!
//! The launcher carries the arg-stamp, so `ps` discovery is the single source
//! of truth for liveness and identity. The JSON cache holds only convenience
//! metadata (log path, start time, server pid) and is always reconciled
//! against `ps`, never trusted over it.

import {
  APP,
  DEFAULT_NAMESPACE,
  readStampFromCommand,
  type Stamp,
  stampArg,
} from "@/lib/dev/stamp.ts";
import { killHard, psList, term } from "@/lib/std/proc.ts";

const BIN_REL = "target/debug/santi-api";
const HEALTH_TIMEOUT_MS = 15_000;
const STOP_TIMEOUT_MS = 15_000;

interface Paths {
  repo: string;
  runDir: string;
  log: string;
  cache: string;
  bin: string;
  wrapper: string;
  denoConfig: string;
}

interface Cache {
  launcherPid: number;
  serverPid?: number;
  port: number;
  log: string;
  startedAt: string;
}

interface Found {
  pid: number;
  stamp: Stamp;
}

export async function run(argv: string[]): Promise<void> {
  const [command, ...rest] = argv;
  let code = 0;
  switch (command) {
    case "start":
      code = await start();
      break;
    case "stop":
      code = await stop();
      break;
    case "restart":
      code = await restart();
      break;
    case "status":
      code = await status();
      break;
    case "logs":
      code = await logs(rest);
      break;
    case "__run":
      code = await launcherRun();
      break;
    default:
      console.error("usage: runseal :dev <start|stop|restart|status|logs>");
      code = 2;
  }
  Deno.exit(code);
}

async function start(): Promise<number> {
  const paths = resolvePaths();
  const running = await discover();
  if (running.length > 0) {
    console.log(`santi-api already running (launcher pid ${running[0].pid})`);
    return 0;
  }
  Deno.mkdirSync(paths.runDir, { recursive: true });

  console.log("building santi-api ...");
  const build = await new Deno.Command("cargo", {
    args: ["build", "-p", "santi-api"],
    cwd: paths.repo,
    stdout: "inherit",
    stderr: "inherit",
  }).output();
  if (!build.success) {
    console.error("build failed");
    return 1;
  }

  const port = readPort(paths.repo);
  const host = readHost(paths.repo);
  const stamp: Stamp = { version: 1, app: APP, namespace: DEFAULT_NAMESPACE, port };

  const launcher = new Deno.Command(Deno.execPath(), {
    args: [
      "run",
      "--no-prompt",
      "--config",
      paths.denoConfig,
      "--allow-run",
      "--allow-read",
      "--allow-write",
      "--allow-env",
      paths.wrapper,
      "__run",
      stampArg(stamp),
    ],
    cwd: paths.repo,
    stdin: "null",
    stdout: "null",
    stderr: "null",
  });
  const child = launcher.spawn();
  child.unref();

  writeCache(paths, {
    launcherPid: child.pid,
    port,
    log: paths.log,
    startedAt: new Date().toISOString(),
  });

  const healthy = await waitHealth(host, port, HEALTH_TIMEOUT_MS);
  if (healthy) {
    console.log(
      `santi-api up on http://${host}:${port} (launcher pid ${child.pid})\nlogs: ${paths.log}`,
    );
  } else {
    console.log(
      `santi-api launched (launcher pid ${child.pid}) but health was not confirmed within ` +
        `${HEALTH_TIMEOUT_MS / 1000}s — inspect: runseal :dev logs`,
    );
  }
  return 0;
}

async function stop(): Promise<number> {
  const paths = resolvePaths();
  const cache = readCache(paths);
  const running = await discover();
  if (running.length === 0) {
    if (cache?.serverPid) term(cache.serverPid); // defensive: clear a possible orphan
    clearCache(paths);
    console.log("santi-api not running");
    return 0;
  }
  for (const found of running) {
    term(found.pid);
  }
  if (!(await waitGone(STOP_TIMEOUT_MS))) {
    for (const found of await discover()) {
      killHard(found.pid);
    }
    await waitGone(3_000);
  }
  if (cache?.serverPid) term(cache.serverPid); // belt-and-suspenders against SIGKILL orphans
  clearCache(paths);
  console.log("santi-api stopped");
  return 0;
}

async function restart(): Promise<number> {
  await stop();
  return await start();
}

async function status(): Promise<number> {
  const paths = resolvePaths();
  const cache = readCache(paths);
  const running = await discover();
  if (running.length === 0) {
    clearCache(paths); // stale cache, if any
    console.log("santi-api: stopped");
    return 0;
  }
  const found = running[0];
  const host = readHost(paths.repo);
  const port = found.stamp.port || cache?.port || readPort(paths.repo);
  const healthy = await waitHealth(host, port, 1_000);
  const uptime = cache?.startedAt
    ? `${Math.round((Date.now() - Date.parse(cache.startedAt)) / 1000)}s`
    : "?";
  console.log("santi-api: running");
  console.log(`  launcher pid : ${found.pid}`);
  if (cache?.serverPid) console.log(`  server pid   : ${cache.serverPid}`);
  console.log(
    `  endpoint     : http://${host}:${port}  (health: ${healthy ? "ok" : "unreachable"})`,
  );
  console.log(`  uptime       : ${uptime}`);
  console.log(`  logs         : ${paths.log}`);
  return 0;
}

async function logs(args: string[]): Promise<number> {
  const paths = resolvePaths();
  const follow = args.includes("-f") || args.includes("--follow");
  let lines = 50;
  const nIndex = args.findIndex((arg) => arg === "-n");
  if (nIndex >= 0 && args[nIndex + 1]) {
    lines = Number(args[nIndex + 1]) || lines;
  }
  if (!exists(paths.log)) {
    console.log(`no log yet: ${paths.log}`);
    return 0;
  }
  if (follow) {
    const child = new Deno.Command("tail", {
      args: ["-n", String(lines), "-f", paths.log],
      stdout: "inherit",
      stderr: "inherit",
    }).spawn();
    return (await child.status).code ?? 0;
  }
  const text = Deno.readTextFileSync(paths.log).split("\n");
  console.log(text.slice(Math.max(0, text.length - lines - 1)).join("\n").trimEnd());
  return 0;
}

/**
 * Launcher mode (internal). Carries the arg-stamp, supervises one santi-api
 * child, pumps its output to the log file, and forwards termination so a kill
 * of the launcher cleanly stops the server.
 */
async function launcherRun(): Promise<number> {
  const paths = resolvePaths();
  // nohup-equivalent: survive a terminal hang-up.
  try {
    Deno.addSignalListener("SIGHUP", () => {});
  } catch {
    // not supported on this platform; ignore
  }

  // Truncate on each launch so the log reflects only the current run.
  const logFile = Deno.openSync(paths.log, { create: true, write: true, truncate: true });

  const server = new Deno.Command(paths.bin, {
    args: ["serve"],
    cwd: paths.repo,
    stdin: "null",
    stdout: "piped",
    stderr: "piped",
  }).spawn();

  const cache = readCache(paths);
  if (cache) writeCache(paths, { ...cache, serverPid: server.pid });

  // Pump with direct `writeSync` rather than `pipeTo`: a FsFile writable stream
  // buffers and only flushes on close, which would hide all logs until the
  // server exits. `writeSync` issues one write syscall per chunk, landing
  // output immediately.
  const pumps = [pump(server.stdout, logFile), pump(server.stderr, logFile)];

  let shuttingDown = false;
  const forward = () => {
    if (shuttingDown) return;
    shuttingDown = true;
    try {
      server.kill("SIGTERM");
    } catch {
      // already gone
    }
  };
  for (const signal of ["SIGTERM", "SIGINT"] as const) {
    try {
      Deno.addSignalListener(signal, forward);
    } catch {
      // ignore
    }
  }

  const result = await server.status;
  await Promise.allSettled(pumps);
  try {
    logFile.close();
  } catch {
    // already closed
  }
  return result.code ?? 0;
}

/** Append a child stream to the log file, flushing each chunk immediately. */
async function pump(stream: ReadableStream<Uint8Array>, file: Deno.FsFile): Promise<void> {
  const reader = stream.getReader();
  try {
    while (true) {
      const { value, done } = await reader.read();
      if (done) break;
      if (value) {
        let offset = 0;
        while (offset < value.length) {
          offset += file.writeSync(value.subarray(offset));
        }
      }
    }
  } catch {
    // stream ended or process gone
  } finally {
    reader.releaseLock();
  }
}

async function discover(): Promise<Found[]> {
  const found: Found[] = [];
  for (const row of await psList()) {
    const stamp = readStampFromCommand(row.command);
    if (stamp && stamp.app === APP && stamp.namespace === DEFAULT_NAMESPACE) {
      found.push({ pid: row.pid, stamp });
    }
  }
  return found;
}

async function waitGone(timeoutMs: number): Promise<boolean> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if ((await discover()).length === 0) return true;
    await sleep(200);
  }
  return (await discover()).length === 0;
}

async function waitHealth(host: string, port: number, timeoutMs: number): Promise<boolean> {
  const deadline = Date.now() + timeoutMs;
  const url = `http://${host}:${port}/api/v1/health`;
  while (Date.now() < deadline) {
    try {
      const response = await fetch(url);
      const ok = response.ok;
      await response.body?.cancel();
      if (ok) return true;
    } catch {
      // not accepting connections yet
    }
    await sleep(300);
  }
  return false;
}

function resolvePaths(): Paths {
  const repo = repoRoot();
  const runDir = join(repo, ".tmp/dev");
  return {
    repo,
    runDir,
    log: join(runDir, "santi-api.log"),
    cache: join(runDir, "santi-api.json"),
    bin: join(repo, BIN_REL),
    wrapper: Deno.env.get("RUNSEAL_WRAPPER_FILE") ?? join(repo, ".runseal/wrappers/dev.ts"),
    denoConfig: join(repo, ".runseal/deno.json"),
  };
}

function repoRoot(): string {
  const profile = Deno.env.get("RUNSEAL_PROFILE_PATH");
  return profile ? dirname(profile) : Deno.cwd();
}

function readPort(repo: string): number {
  const fromEnv = Deno.env.get("SANTI_PORT");
  if (fromEnv && Number.isInteger(Number(fromEnv))) return Number(fromEnv);
  const fromFile = readEnvFile(repo, "SANTI_PORT");
  if (fromFile && Number.isInteger(Number(fromFile))) return Number(fromFile);
  return 43307;
}

function readHost(repo: string): string {
  return Deno.env.get("SANTI_HOST") ?? readEnvFile(repo, "SANTI_HOST") ?? "127.0.0.1";
}

function readEnvFile(repo: string, key: string): string | null {
  try {
    const text = Deno.readTextFileSync(join(repo, ".env"));
    const match = text.match(new RegExp(`^\\s*${key}\\s*=\\s*(\\S+)\\s*$`, "m"));
    return match ? match[1] : null;
  } catch {
    return null;
  }
}

function readCache(paths: Paths): Cache | null {
  try {
    return JSON.parse(Deno.readTextFileSync(paths.cache)) as Cache;
  } catch {
    return null;
  }
}

function writeCache(paths: Paths, cache: Cache): void {
  Deno.mkdirSync(paths.runDir, { recursive: true });
  Deno.writeTextFileSync(paths.cache, `${JSON.stringify(cache, null, 2)}\n`);
}

function clearCache(paths: Paths): void {
  try {
    Deno.removeSync(paths.cache);
  } catch {
    // already absent
  }
}

function exists(path: string): boolean {
  try {
    Deno.statSync(path);
    return true;
  } catch {
    return false;
  }
}

function dirname(path: string): string {
  const trimmed = path.replace(/\/+$/, "");
  const index = trimmed.lastIndexOf("/");
  return index <= 0 ? "/" : trimmed.slice(0, index);
}

function join(...parts: string[]): string {
  return parts.join("/").replace(/(?<!:)\/{2,}/g, "/");
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
