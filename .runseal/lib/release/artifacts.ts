//! Build, archive, checksum, and validate release artifacts for one `santi`
//! binary per target.

import { capture, run } from "@/lib/std/cmd.ts";
import { exists, join } from "@/lib/std/fs.ts";
import { fail, required } from "@/lib/release/env.ts";

export interface Artifact {
  target: string;
  archive: string;
  member: string;
  metadataKey: string;
  contentType: string;
}

export const ARTIFACTS: Artifact[] = [
  {
    target: "x86_64-unknown-linux-gnu",
    archive: "santi-x86_64-unknown-linux-gnu.tar.gz",
    member: "santi",
    metadataKey: "linuxX64",
    contentType: "application/gzip",
  },
  {
    target: "aarch64-apple-darwin",
    archive: "santi-aarch64-apple-darwin.tar.gz",
    member: "santi",
    metadataKey: "macArm64",
    contentType: "application/gzip",
  },
  {
    target: "x86_64-apple-darwin",
    archive: "santi-x86_64-apple-darwin.tar.gz",
    member: "santi",
    metadataKey: "macX64",
    contentType: "application/gzip",
  },
  {
    target: "x86_64-pc-windows-msvc",
    archive: "santi-x86_64-pc-windows-msvc.zip",
    member: "santi.exe",
    metadataKey: "winX64",
    contentType: "application/zip",
  },
];

export function artifactDir(repo: string, version: string): string {
  return join(repo, "dist", version);
}

/** Build `santi` for $TARGET (or the host) and archive it into dist/<version>/. */
export async function pkg(repo: string): Promise<void> {
  const version = required("RELEASE_VERSION");
  const target = (Deno.env.get("TARGET") ?? "").trim() || (await hostTarget());
  const spec = ARTIFACTS.find((a) => a.target === target);
  if (!spec) fail(`unsupported target: ${target}`);

  const dir = artifactDir(repo, version);
  Deno.mkdirSync(dir, { recursive: true });

  if (
    await run("cargo", ["build", "--release", "--locked", "-p", "santi", "--target", target], {
      cwd: repo,
    }) !== 0
  ) {
    fail(`cargo build failed for ${target}`);
  }

  const bin = join(repo, "target", target, "release", spec.member);
  const stage = await Deno.makeTempDir();
  await Deno.copyFile(bin, join(stage, spec.member));
  if (!spec.member.endsWith(".exe")) {
    try {
      Deno.chmodSync(join(stage, spec.member), 0o755);
    } catch {
      // non-unix; ignore
    }
  }

  const out = join(dir, spec.archive);
  const code = spec.archive.endsWith(".tar.gz")
    ? await run("tar", ["-C", stage, "-czf", out, spec.member])
    : await run("tar", ["-C", stage, "-a", "-c", "-f", out, spec.member]); // bsdtar (Windows) → zip
  if (code !== 0) fail(`archiving ${spec.archive} failed`);
  console.log(out);
}

/** Write checksums.txt (VERSION header + sha256 of each archive). */
export async function checksums(repo: string): Promise<void> {
  const version = required("RELEASE_VERSION");
  const dir = artifactDir(repo, version);
  const lines = [`VERSION: ${version}`];
  for (const spec of ARTIFACTS) {
    const path = join(dir, spec.archive);
    if (!exists(path)) continue;
    lines.push(`${await sha256(path)}  ${spec.archive}`);
  }
  Deno.writeTextFileSync(join(dir, "checksums.txt"), `${lines.join("\n")}\n`);
}

/** Require all archives + checksums present, the version line, and entries. */
export function accept(repo: string): void {
  const version = required("RELEASE_VERSION");
  const dir = artifactDir(repo, version);
  if (!exists(join(dir, "checksums.txt"))) fail("missing checksums.txt");
  for (const spec of ARTIFACTS) {
    if (!exists(join(dir, spec.archive))) fail(`missing artifact: ${spec.archive}`);
  }
  const text = Deno.readTextFileSync(join(dir, "checksums.txt"));
  const header = text.split("\n").find((l) => l.startsWith("VERSION:"));
  const value = header?.replace(/^VERSION:\s*/, "").trim();
  if (value !== version) fail(`version mismatch: expected ${version} got ${value ?? "<none>"}`);
  for (const spec of ARTIFACTS) {
    if (!text.split("\n").some((l) => l.trim().endsWith(` ${spec.archive}`))) {
      fail(`missing checksum entry: ${spec.archive}`);
    }
  }
}

/** Confirm each archive actually contains the expected binary member. */
export async function verifyMembers(repo: string): Promise<void> {
  const version = required("RELEASE_VERSION");
  const dir = artifactDir(repo, version);
  for (const spec of ARTIFACTS) {
    const path = join(dir, spec.archive);
    const names = spec.archive.endsWith(".tar.gz")
      ? (await capture("tar", ["-tzf", path])).stdout
      : (await capture("unzip", ["-Z1", path])).stdout;
    if (!names.split("\n").map((n) => n.trim()).includes(spec.member)) {
      fail(`missing ${spec.member} in ${spec.archive}`);
    }
  }
}

async function hostTarget(): Promise<string> {
  const result = await capture("rustc", ["-Vv"]);
  const match = result.stdout.match(/^host:\s*(.+)$/m);
  if (!match) fail("could not determine host target from rustc -Vv");
  return match[1].trim();
}

async function sha256(path: string): Promise<string> {
  const data = await Deno.readFile(path);
  const digest = await crypto.subtle.digest("SHA-256", data);
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
}
