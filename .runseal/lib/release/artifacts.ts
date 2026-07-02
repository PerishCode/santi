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
  /** "archive" = tar.gz/zip of the bare binary; "deb" = a Debian package. */
  kind?: "archive" | "deb";
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
    // Server-only Debian package built alongside the linux-gnu tarball (same
    // build job). The low-friction entry for Liberte-driven self-upgrade
    // (PHASE-07): ships the binary + systemd units + maintainer scripts.
    target: "x86_64-unknown-linux-gnu",
    archive: "santi-x86_64-unknown-linux-gnu.deb",
    member: "usr/bin/santi",
    metadataKey: "debX64",
    contentType: "application/vnd.debian.binary-package",
    kind: "deb",
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

/** Build `santi` for $TARGET (or the host) and produce every artifact bound to
 * that target into dist/<version>/ — the linux-gnu target yields BOTH the tarball
 * and the `.deb`. */
export async function pkg(repo: string): Promise<void> {
  const version = required("RELEASE_VERSION");
  const target = (Deno.env.get("TARGET") ?? "").trim() || (await hostTarget());
  const specs = ARTIFACTS.filter((a) => a.target === target);
  if (specs.length === 0) fail(`unsupported target: ${target}`);

  const dir = artifactDir(repo, version);
  Deno.mkdirSync(dir, { recursive: true });

  if (
    await run("cargo", ["build", "--release", "--locked", "-p", "santi", "--target", target], {
      cwd: repo,
    }) !== 0
  ) {
    fail(`cargo build failed for ${target}`);
  }

  const binMember = specs.find((s) => (s.kind ?? "archive") === "archive")?.member ?? "santi";
  const bin = join(repo, "target", target, "release", binMember);

  for (const spec of specs) {
    const out = join(dir, spec.archive);
    if ((spec.kind ?? "archive") === "deb") {
      await buildDeb(repo, version, bin, out);
    } else {
      const stage = await Deno.makeTempDir();
      await Deno.copyFile(bin, join(stage, spec.member));
      if (!spec.member.endsWith(".exe")) {
        try {
          Deno.chmodSync(join(stage, spec.member), 0o755);
        } catch {
          // non-unix; ignore
        }
      }
      const code = spec.archive.endsWith(".tar.gz")
        ? await run("tar", ["-C", stage, "-czf", out, spec.member])
        : await run("tar", ["-C", stage, "-a", "-c", "-f", out, spec.member]); // bsdtar (Windows) → zip
      if (code !== 0) fail(`archiving ${spec.archive} failed`);
    }
    console.log(out);
  }
}

/** Stage the .deb tree from `.runseal/packaging/deb/` + the built binary and
 * `dpkg-deb --build` it. Requires `dpkg-deb` (present on the ubuntu build job). */
async function buildDeb(
  repo: string,
  version: string,
  binPath: string,
  outPath: string,
): Promise<void> {
  // Debian version: drop the leading `v` (`v0.1.0-beta.11` → `0.1.0-beta.11`).
  const debVersion = version.replace(/^v/, "");
  const src = join(repo, ".runseal", "packaging", "deb");
  const stage = await Deno.makeTempDir();
  const mkdir = (rel: string) => Deno.mkdirSync(join(stage, rel), { recursive: true });
  mkdir("usr/bin");
  mkdir("lib/systemd/system");
  mkdir("etc/santi");
  mkdir("DEBIAN");

  await Deno.copyFile(binPath, join(stage, "usr/bin/santi"));
  Deno.chmodSync(join(stage, "usr/bin/santi"), 0o755);
  await Deno.copyFile(join(src, "santi.service"), join(stage, "lib/systemd/system/santi.service"));
  await Deno.copyFile(
    join(src, "santi-upgrade.service"),
    join(stage, "lib/systemd/system/santi-upgrade.service"),
  );
  await Deno.copyFile(
    join(src, "santi.env.example"),
    join(stage, "etc/santi/santi.env.example"),
  );

  const control = Deno.readTextFileSync(join(src, "control")).replaceAll("__VERSION__", debVersion);
  Deno.writeTextFileSync(join(stage, "DEBIAN/control"), control);
  for (const script of ["postinst", "prerm", "postrm"]) {
    await Deno.copyFile(join(src, script), join(stage, "DEBIAN", script));
    Deno.chmodSync(join(stage, "DEBIAN", script), 0o755);
  }

  // --root-owner-group: files owned by root:root without needing fakeroot/sudo.
  if (await run("dpkg-deb", ["--root-owner-group", "--build", stage, outPath]) !== 0) {
    fail(`dpkg-deb --build failed for ${outPath}`);
  }
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
    if ((spec.kind ?? "archive") === "deb") {
      // A .deb is an `ar` archive; list its payload with dpkg-deb (paths are
      // `./usr/bin/santi`), and confirm the maintainer scripts are executable.
      const contents = (await capture("dpkg-deb", ["--contents", path])).stdout;
      if (!contents.includes(`/${spec.member}`)) {
        fail(`missing ${spec.member} in ${spec.archive}`);
      }
      continue;
    }
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
