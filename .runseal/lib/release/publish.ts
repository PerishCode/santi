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
  // The Linux install manager lives at the root and is refreshed every release.
  await putObject(join(repo, "manage.sh"), "manage.sh", contentTypeFor("manage.sh"), REVALIDATE);

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

  const artifacts: Record<string, unknown> = Object.fromEntries(
    ARTIFACTS.map((spec) => [spec.metadataKey, artifact(spec.archive, spec.contentType)]),
  );
  artifacts.checksums = artifact("checksums.txt", "text/plain; charset=utf-8");

  const metadata: Record<string, unknown> = {
    version: 1,
    channel,
    releaseVersion: version,
    generatedAt: new Date().toISOString().replace(/\.\d+Z$/, "Z"),
    ci: {
      repository: Deno.env.get("CI_REPOSITORY") ?? "",
      commit: Deno.env.get("CI_COMMIT") ?? "",
      runId: Number(Deno.env.get("CI_RUN_ID") ?? 0),
      runAttempt: Number(Deno.env.get("CI_RUN_ATTEMPT") ?? 0),
      workflow: Deno.env.get("CI_WORKFLOW") ?? "",
    },
    r2: {
      publicUrl: pub,
      latestMetadataUrl: `${pub}/${latestPrefix}/metadata.json`,
      versionMetadataUrl: `${pub}/${versionPrefix}/metadata.json`,
      versionPrefix,
      latestPrefix,
    },
    manage: { unix: `${pub}/manage.sh` },
    artifacts,
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

  const run = Deno.env.get("CI_RUN_ID") ?? "local";
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
  ];
  for (const url of urls) {
    const head = await fetch(url, { method: "HEAD" });
    await head.body?.cancel();
    if (!head.ok) fail(`HEAD ${url} -> HTTP ${head.status}`);
  }
  console.log(`[release] verified ${urls.length} published URLs for ${version}`);
}
