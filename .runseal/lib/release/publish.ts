//! Publish artifacts + metadata.json to R2, and verify the published result.

import { exists, join } from "@/lib/std/fs.ts";
import { appendSummary, fail, publicUrl, required, writeOutput } from "@/lib/release/env.ts";
import { artifactDir, ARTIFACTS } from "@/lib/release/artifacts.ts";
import { contentTypeFor, putObject } from "@/lib/release/r2.ts";

const IMMUTABLE = "public, max-age=31536000, immutable";
const REVALIDATE = "public, max-age=60, must-revalidate";

export async function publish(repo: string): Promise<void> {
  const channel = required("RELEASE_CHANNEL");
  const version = required("RELEASE_VERSION");
  const dir = artifactDir(repo, version);
  const versionPrefix = `${channel}/versions/${version}`;
  const latestPrefix = `${channel}/latest`;
  const pub = publicUrl();

  for (const spec of ARTIFACTS) {
    await putObject(
      join(dir, spec.archive),
      `${versionPrefix}/${spec.archive}`,
      spec.contentType,
      IMMUTABLE,
    );
  }
  await putObject(
    join(dir, "checksums.txt"),
    `${versionPrefix}/checksums.txt`,
    contentTypeFor("checksums.txt"),
    IMMUTABLE,
  );

  // Public install managers live at the root and are refreshed every release
  // (same script regardless of channel), so installs work on any channel.
  await putObject(join(repo, "manage.sh"), "manage.sh", contentTypeFor("manage.sh"), REVALIDATE);
  await putObject(join(repo, "manage.ps1"), "manage.ps1", contentTypeFor("manage.ps1"), REVALIDATE);

  const metadata = buildMetadata(channel, version, versionPrefix, latestPrefix, pub, dir);
  const metaPath = join(dir, "metadata.json");
  Deno.writeTextFileSync(metaPath, `${JSON.stringify(metadata, null, 2)}\n`);
  await putObject(metaPath, `${versionPrefix}/metadata.json`, contentTypeFor(".json"), IMMUTABLE);
  await putObject(metaPath, `${latestPrefix}/metadata.json`, contentTypeFor(".json"), REVALIDATE);

  writeOutput("metadata_url", `${pub}/${latestPrefix}/metadata.json`);
  writeOutput("version_metadata_url", `${pub}/${versionPrefix}/metadata.json`);
  writeOutput("version_prefix", versionPrefix);

  appendSummary([
    `## santi ${channel} release`,
    "",
    `- Version: \`${version}\``,
    `- R2 prefix: \`${versionPrefix}\``,
    `- Latest metadata: ${pub}/${latestPrefix}/metadata.json`,
    `- Unix manager: ${pub}/manage.sh`,
  ]);
}

function buildMetadata(
  channel: string,
  version: string,
  versionPrefix: string,
  latestPrefix: string,
  pub: string,
  dir: string,
): Record<string, unknown> {
  const artifact = (name: string, contentType: string) => {
    const path = join(dir, name);
    if (!exists(path)) fail(`missing metadata source file: ${path}`);
    return {
      contentType,
      name,
      size: Deno.statSync(path).size,
      url: `${pub}/${versionPrefix}/${name}`,
    };
  };

  const metadata: Record<string, unknown> = {
    version: 1,
    channel,
    releaseVersion: version,
    generatedAt: new Date().toISOString().replace(/\.\d+Z$/, "Z"),
    github: {
      repository: Deno.env.get("GITHUB_REPOSITORY") ?? "",
      commit: Deno.env.get("GITHUB_SHA") ?? "",
      runId: Number(Deno.env.get("GITHUB_RUN_ID") ?? 0),
      runAttempt: Number(Deno.env.get("GITHUB_RUN_ATTEMPT") ?? 0),
      workflow: Deno.env.get("GITHUB_WORKFLOW") ?? "",
    },
    r2: {
      publicUrl: pub,
      latestMetadataUrl: `${pub}/${latestPrefix}/metadata.json`,
      versionMetadataUrl: `${pub}/${versionPrefix}/metadata.json`,
      versionPrefix,
      latestPrefix,
    },
    manage: { unix: `${pub}/manage.sh`, windows: `${pub}/manage.ps1` },
    artifacts: {
      linuxX64: artifact("santi-x86_64-unknown-linux-gnu.tar.gz", "application/gzip"),
      debX64: artifact(
        "santi-x86_64-unknown-linux-gnu.deb",
        "application/vnd.debian.binary-package",
      ),
      macArm64: artifact("santi-aarch64-apple-darwin.tar.gz", "application/gzip"),
      macX64: artifact("santi-x86_64-apple-darwin.tar.gz", "application/gzip"),
      winX64: artifact("santi-x86_64-pc-windows-msvc.zip", "application/zip"),
      checksums: artifact("checksums.txt", "text/plain; charset=utf-8"),
    },
  };

  if (channel === "beta") {
    const m = /^v?(\d+\.\d+\.\d+)-beta\.([1-9][0-9]*)$/.exec(version);
    if (!m) fail(`invalid beta release version: ${version}`);
    metadata.baseVersion = Deno.env.get("BASE_VERSION") || m[1];
    metadata.betaNumber = Number(Deno.env.get("BETA_NUMBER") || m[2]);
    metadata.betaVersion = version;
    metadata.stateSource = Deno.env.get("STATE_SOURCE") || "workflow input";
  } else {
    metadata.stableVersion = version;
    metadata.stateSource = Deno.env.get("STATE_SOURCE") || "workflow input";
  }
  return metadata;
}

export async function verifyPublish(): Promise<void> {
  const channel = required("RELEASE_CHANNEL");
  const version = required("RELEASE_VERSION");
  const metadataUrl = required("R2_METADATA_URL");
  const pub = publicUrl();

  const run = Deno.env.get("GITHUB_RUN_ID") ?? "local";
  const response = await fetch(`${metadataUrl}?run=${run}`);
  if (!response.ok) fail(`failed to fetch published metadata: HTTP ${response.status}`);
  // deno-lint-ignore no-explicit-any
  const metadata = (await response.json()) as any;

  if (metadata.channel !== channel) fail(`unexpected channel: ${metadata.channel}`);
  if (metadata.releaseVersion !== version) {
    fail(`unexpected releaseVersion: ${metadata.releaseVersion}`);
  }
  if (metadata.manage?.unix !== `${pub}/manage.sh`) {
    fail(`unexpected unix manager url: ${metadata.manage?.unix}`);
  }
  if (metadata.manage?.windows !== `${pub}/manage.ps1`) {
    fail(`unexpected windows manager url: ${metadata.manage?.windows}`);
  }
  if (channel === "beta") {
    if (metadata.betaVersion !== version) fail(`unexpected betaVersion: ${metadata.betaVersion}`);
    const base = metadata.baseVersion;
    const number = metadata.betaNumber;
    if (typeof base !== "string" || !base) fail("missing baseVersion");
    if (typeof number !== "number") fail("missing betaNumber");
    if (`v${base}-beta.${number}` !== version) {
      fail("beta metadata does not reconstruct the release version");
    }
  }

  const urls: string[] = [
    ...Object.values(metadata.artifacts).map((item) => (item as { url: string }).url),
    metadata.manage.unix,
    metadata.manage.windows,
  ];
  for (const url of urls) {
    const head = await fetch(url, { method: "HEAD" });
    await head.body?.cancel();
    if (!head.ok) fail(`HEAD ${url} -> HTTP ${head.status}`);
  }
  console.log(`[release] verified ${urls.length} published URLs for ${version}`);
}
