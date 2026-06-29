//! Release version resolver. The remote `<channel>/latest/metadata.json` in R2
//! is the source of truth; the next version is computed from it plus the
//! workspace Cargo.toml version, with strict monotonic rules. Ported from
//! flavor's beta.py / stable.py.

import { fail, publicUrl, writeOutput } from "@/lib/release/env.ts";

const BOOTSTRAP_404_RETRY_MS = 15_000;
const STABLE_RE = /^(\d+)\.(\d+)\.(\d+)$/;
const BETA_RE = /^v?(\d+\.\d+\.\d+)-beta\.([1-9][0-9]*)$/;
const TAGGED_STABLE_RE = /^v?(\d+\.\d+\.\d+)$/;

export type Channel = "beta" | "stable";

export async function resolveVersion(channel: Channel, repo: string): Promise<number> {
  const cargo = cargoVersion(repo);
  const override = (Deno.env.get(`${channel.toUpperCase()}_VERSION_OVERRIDE`) ?? "").trim();

  if (channel === "beta") {
    const resolved = override ? betaFromOverride(override, cargo) : await nextBeta(cargo);
    emit(resolved);
  } else {
    const resolved = override ? stableFromOverride(override, cargo) : await nextStable(cargo);
    emit(resolved);
  }
  return 0;
}

interface Resolved {
  channel: Channel;
  baseVersion: string;
  releaseVersion: string;
  betaNumber?: number;
  stateSource: string;
}

function emit(r: Resolved): void {
  console.log(`[release-${r.channel}] channel: ${r.channel}`);
  console.log(`[release-${r.channel}] base version: ${r.baseVersion}`);
  if (r.betaNumber !== undefined) {
    console.log(`[release-${r.channel}] beta number: ${r.betaNumber}`);
  }
  console.log(`[release-${r.channel}] release version: ${r.releaseVersion}`);
  console.log(`[release-${r.channel}] state source: ${r.stateSource}`);
  writeOutput("base_version", r.baseVersion);
  writeOutput("release_version", r.releaseVersion);
  if (r.betaNumber !== undefined) writeOutput("beta_number", String(r.betaNumber));
  writeOutput("state_source", r.stateSource);
}

async function nextBeta(cargo: string): Promise<Resolved> {
  const url = `${publicUrl()}/beta/latest/metadata.json`;
  console.log(`[release-beta] metadata url: ${url}`);
  const text = await fetchOptional(url, "beta");
  if (text === null) {
    return beta(cargo, 1, "missing R2 beta metadata");
  }
  const metadata = parseJson(text, "beta");
  const [base, number] = readBeta(metadata);
  const order = cmp(cargo, base);
  if (order < 0) fail(`Cargo version ${cargo} regressed below beta base ${base}`);
  if (order > 0) return beta(cargo, 1, "R2 beta metadata base advanced");
  return beta(cargo, number + 1, `R2 beta metadata v${base}-beta.${number}`);
}

async function nextStable(cargo: string): Promise<Resolved> {
  const url = `${publicUrl()}/stable/latest/metadata.json`;
  console.log(`[release-stable] metadata url: ${url}`);
  const text = await fetchOptional(url, "stable");
  if (text === null) {
    return stable(cargo, "missing R2 stable metadata");
  }
  const metadata = parseJson(text, "stable");
  const prior = readStable(metadata);
  const order = cmp(cargo, prior);
  if (order < 0) fail(`Cargo version ${cargo} regressed below prior stable ${prior}`);
  if (order === 0) {
    fail(`Cargo version ${cargo} matches the prior stable; bump Cargo.toml before re-running`);
  }
  return stable(cargo, `R2 stable metadata v${prior}`);
}

function beta(base: string, number: number, stateSource: string): Resolved {
  return {
    channel: "beta",
    baseVersion: base,
    betaNumber: number,
    releaseVersion: `v${base}-beta.${number}`,
    stateSource,
  };
}

function stable(base: string, stateSource: string): Resolved {
  return { channel: "stable", baseVersion: base, releaseVersion: `v${base}`, stateSource };
}

function betaFromOverride(override: string, cargo: string): Resolved {
  const m = BETA_RE.exec(override);
  if (!m) fail(`BETA_VERSION_OVERRIDE must look like vX.Y.Z-beta.N, got ${override}`);
  if (m[1] !== cargo) fail(`override base ${m[1]} does not match Cargo version ${cargo}`);
  return beta(m[1], Number(m[2]), "workflow override");
}

function stableFromOverride(override: string, cargo: string): Resolved {
  const m = TAGGED_STABLE_RE.exec(override);
  if (!m) fail(`STABLE_VERSION_OVERRIDE must look like vX.Y.Z, got ${override}`);
  if (m[1] !== cargo) fail(`override base ${m[1]} does not match Cargo version ${cargo}`);
  return stable(m[1], "workflow override");
}

function readBeta(metadata: Record<string, unknown>): [string, number] {
  const direct = metadata.betaVersion ?? metadata.releaseVersion;
  if (typeof direct === "string" && direct) {
    const m = BETA_RE.exec(direct);
    if (!m) fail(`R2 beta metadata must look like vX.Y.Z-beta.N, got ${direct}`);
    return [m[1], Number(m[2])];
  }
  const base = metadata.baseVersion;
  const number = metadata.betaNumber;
  if (typeof base === "string" && typeof number === "number") {
    versionTuple(base);
    if (number < 1) fail(`R2 beta metadata betaNumber must be >= 1, got ${number}`);
    return [base, number];
  }
  fail("R2 beta metadata must include betaVersion or releaseVersion");
}

function readStable(metadata: Record<string, unknown>): string {
  const direct = metadata.stableVersion ?? metadata.releaseVersion;
  if (typeof direct === "string" && direct) {
    const m = TAGGED_STABLE_RE.exec(direct);
    if (!m) fail(`R2 stable metadata must look like vX.Y.Z, got ${direct}`);
    return m[1];
  }
  const base = metadata.baseVersion;
  if (typeof base === "string") {
    versionTuple(base);
    return base;
  }
  fail("R2 stable metadata must include stableVersion, releaseVersion, or baseVersion");
}

function cargoVersion(repo: string): string {
  const text = Deno.readTextFileSync(`${repo}/Cargo.toml`);
  const m = text.match(/^version = "([^"]+)"$/m);
  if (!m) fail("missing [workspace.package] version in Cargo.toml");
  versionTuple(m[1]);
  return m[1];
}

function versionTuple(value: string): [number, number, number] {
  const m = STABLE_RE.exec(value);
  if (!m) fail(`expected stable x.y.z version, got ${value}`);
  return [Number(m[1]), Number(m[2]), Number(m[3])];
}

function cmp(a: string, b: string): number {
  const x = versionTuple(a);
  const y = versionTuple(b);
  for (let i = 0; i < 3; i++) {
    if (x[i] !== y[i]) return x[i] < y[i] ? -1 : 1;
  }
  return 0;
}

function parseJson(text: string, channel: Channel): Record<string, unknown> {
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch (error) {
    fail(
      `R2 ${channel} metadata is invalid JSON: ${error instanceof Error ? error.message : error}`,
    );
  }
  if (typeof value !== "object" || value === null) {
    fail(`R2 ${channel} metadata must be a JSON object`);
  }
  return value as Record<string, unknown>;
}

async function fetchOptional(url: string, channel: Channel): Promise<string | null> {
  let result = await tryFetch(url, channel);
  if (result.text !== undefined) return result.text;
  if (result.code === 403) {
    fail(`R2 ${channel} metadata returned 403; permission errors must not be treated as missing`);
  }
  if (result.code === 404) {
    console.log(
      `[release-${channel}] R2 metadata 404; retrying after ${
        BOOTSTRAP_404_RETRY_MS / 1000
      }s to confirm absence`,
    );
    await new Promise((resolve) => setTimeout(resolve, BOOTSTRAP_404_RETRY_MS));
    result = await tryFetch(url, channel);
    if (result.text !== undefined) return result.text;
    if (result.code === 403) {
      fail(`R2 ${channel} metadata 403 on retry; refusing to bootstrap on permission error`);
    }
    if (result.code === 404) return null;
  }
  fail(`failed to fetch R2 ${channel} metadata: HTTP ${result.code}`);
}

async function tryFetch(
  url: string,
  channel: Channel,
): Promise<{ text?: string; code?: number }> {
  let response: Response;
  try {
    response = await fetch(url, {
      headers: { "Cache-Control": "no-cache", "User-Agent": `santi-release-${channel}/1.0` },
    });
  } catch (error) {
    fail(
      `failed to fetch R2 ${channel} metadata: ${error instanceof Error ? error.message : error}`,
    );
  }
  if (response.ok) return { text: await response.text() };
  await response.body?.cancel();
  return { code: response.status };
}
