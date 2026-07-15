//! Source-bound freshness manifest for the embedded web bundle (D4C/D5,
//! Liberte 2026-07-15).
//!
//! The web build writes `web/dist.manifest`: a canonical digest over every
//! build-relevant input (sources, configs, lockfile, pins, and this file —
//! the recipe itself) plus a canonical digest over the produced `web/dist`
//! tree, with the actual node/pnpm versions verified against the pins. The
//! cargo embed (W-B) recomputes both sides independently before embedding;
//! a stale dist cannot ride in on its own stale-but-matching output digest.
//! The tree-digest algorithm is shared with Rust through the golden vectors
//! in `vectors.json`.

import { capture } from "@/lib/std/cmd.ts";
import { join } from "@/lib/std/fs.ts";

export const ALGORITHM = "sha256-tree-v1";
export const MANIFEST_SCHEMA = "santi.web_dist.v1";
export const MANIFEST_FILE = "web/dist.manifest";

export interface Manifest {
  schema: string;
  algorithm: string;
  inputs: string;
  output: string;
  node: string;
  pnpm: string;
}

export interface Entry {
  path: string;
  bytes: Uint8Array;
}

/**
 * Canonical tree digest: entries sorted bytewise by relative forward-slash
 * path; one line `<sha256(content)>  <path>\n` per file; digest = sha256 of
 * the concatenation. Empty, dot, parent, backslashed, absolute, or duplicate
 * paths are rejected — normalization is the caller's proof, not a repair.
 */
export async function treeDigest(entries: Entry[]): Promise<string> {
  const seen = new Set<string>();
  for (const entry of entries) {
    const path = entry.path;
    if (path.length === 0 || path.startsWith("/") || path.includes("\\")) {
      throw new Error(`tree digest: unnormalized path: "${path}"`);
    }
    for (const segment of path.split("/")) {
      if (segment === "" || segment === "." || segment === "..") {
        throw new Error(`tree digest: unnormalized path segment in "${path}"`);
      }
    }
    if (seen.has(path)) {
      throw new Error(`tree digest: duplicate path "${path}"`);
    }
    seen.add(path);
  }
  const sorted = [...entries].sort((left, right) =>
    left.path < right.path ? -1 : left.path > right.path ? 1 : 0
  );
  let lines = "";
  for (const entry of sorted) {
    lines += `${await sha256(entry.bytes)}  ${entry.path}\n`;
  }
  return await sha256(new TextEncoder().encode(lines));
}

/** Walk a tree into digest entries; symlinks and non-regular entries fail. */
export async function collect(root: string, base: string): Promise<Entry[]> {
  const entries: Entry[] = [];
  const walk = async (relative: string) => {
    for await (const item of Deno.readDir(join(root, base, relative))) {
      const path = relative === "" ? item.name : `${relative}/${item.name}`;
      if (item.isSymlink || (!item.isDirectory && !item.isFile)) {
        throw new Error(`${base}/${path}: symlinks and non-regular entries are rejected`);
      }
      if (item.isDirectory) {
        await walk(path);
      } else {
        entries.push({ path, bytes: await Deno.readFile(join(root, base, path)) });
      }
    }
  };
  await walk("");
  return entries;
}

/** Build-relevant recipe inputs, spelled from the repo root. */
export const RECIPE = [
  "web/index.html",
  "web/vite.config.ts",
  "web/tsconfig.json",
  "web/package.json",
  "web/pnpm-workspace.yaml",
  "web/pnpm-lock.yaml",
  "web/.node-version",
  ".runseal/lib/web/manifest.ts",
];

export async function inputEntries(root: string): Promise<Entry[]> {
  const entries: Entry[] = [];
  for (const source of ["web/src", "web/public"]) {
    try {
      for (const entry of await collect(root, source)) {
        entries.push({ path: `${source}/${entry.path}`, bytes: entry.bytes });
      }
    } catch (error) {
      if (!(error instanceof Deno.errors.NotFound)) {
        throw error;
      }
    }
  }
  for (const path of RECIPE) {
    entries.push({ path, bytes: await Deno.readFile(join(root, path)) });
  }
  return entries;
}

/** Actual toolchain versions, verified against the repository pins. */
export async function pins(root: string): Promise<{ node: string; pnpm: string }> {
  const nodePin = (await Deno.readTextFile(join(root, "web/.node-version"))).trim();
  const pkg = JSON.parse(await Deno.readTextFile(join(root, "web/package.json"))) as {
    packageManager?: string;
    engines?: { node?: string };
  };
  const pnpmPin = (pkg.packageManager ?? "").replace("pnpm@", "");
  if (pnpmPin.length === 0) {
    throw new Error("web/package.json: packageManager pnpm pin is missing");
  }
  if (pkg.engines?.node !== nodePin) {
    throw new Error(
      `web pins disagree: engines.node ${pkg.engines?.node} != .node-version ${nodePin}`,
    );
  }
  const node = (await capture("node", ["--version"])).stdout.trim();
  if (node !== `v${nodePin}`) {
    throw new Error(`node ${node} does not match the pin v${nodePin}`);
  }
  const pnpm = (await capture("pnpm", ["--version"])).stdout.trim();
  if (pnpm !== pnpmPin) {
    throw new Error(`pnpm ${pnpm} does not match the pin ${pnpmPin}`);
  }
  return { node, pnpm };
}

/** Generate `web/dist.manifest` from the freshly built tree. */
export async function generate(root: string): Promise<Manifest> {
  const { node, pnpm } = await pins(root);
  const dist = await collect(root, "web/dist");
  if (!dist.some((entry) => entry.path === "index.html")) {
    throw new Error("web/dist has no index.html; run the web build first");
  }
  const manifest: Manifest = {
    schema: MANIFEST_SCHEMA,
    algorithm: ALGORITHM,
    inputs: await treeDigest(await inputEntries(root)),
    output: await treeDigest(dist),
    node,
    pnpm,
  };
  await Deno.writeTextFile(
    join(root, MANIFEST_FILE),
    `${JSON.stringify(manifest, null, 2)}\n`,
  );
  return manifest;
}

async function sha256(bytes: Uint8Array): Promise<string> {
  const hash = await crypto.subtle.digest("SHA-256", bytes as BufferSource);
  return Array.from(new Uint8Array(hash))
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("");
}
