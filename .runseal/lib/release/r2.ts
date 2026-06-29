//! Cloudflare R2 (S3-compatible) access, via the `aws` CLI. Credentials are
//! passed through the environment, never logged.

import { capture } from "@/lib/std/cmd.ts";
import { fail, required } from "@/lib/release/env.ts";

function awsEnv(): Record<string, string> {
  return {
    AWS_ACCESS_KEY_ID: required("SANTI_RELEASES_S3_AK"),
    AWS_SECRET_ACCESS_KEY: required("SANTI_RELEASES_S3_SK"),
    AWS_DEFAULT_REGION: "auto",
    AWS_EC2_METADATA_DISABLED: "true",
  };
}

function endpoint(): string {
  return required("SANTI_RELEASES_S3_URL").replace(/\/+$/, "");
}

async function aws(args: string[]) {
  return await capture("aws", ["--endpoint-url", endpoint(), ...args], { env: awsEnv() });
}

export async function putObject(
  filePath: string,
  key: string,
  contentType: string,
  cacheControl: string,
): Promise<void> {
  const result = await aws([
    "s3api",
    "put-object",
    "--bucket",
    required("SANTI_RELEASES_S3_BUCKET"),
    "--key",
    key,
    "--body",
    filePath,
    "--content-type",
    contentType,
    "--cache-control",
    cacheControl,
    "--no-cli-pager",
  ]);
  if (result.code !== 0) {
    fail(`put-object ${key} failed: ${(result.stderr || result.stdout).trim()}`);
  }
}

/** Write a small probe object to confirm the credentials can write. */
export async function accessCheck(channel: string): Promise<void> {
  const probe = required("R2_ACCESS_PROBE_NAME");
  const key = `${channel}/.ci-access-check/${probe}.txt`;
  const file = await Deno.makeTempFile({ suffix: ".txt" });
  await Deno.writeTextFile(file, `channel=${channel}\nts=${new Date().toISOString()}\n`);
  try {
    await putObject(file, key, "text/plain; charset=utf-8", "no-store");
    console.log(`[release] R2 write access ok (${key})`);
  } finally {
    await Deno.remove(file).catch(() => {});
  }
}

export function contentTypeFor(name: string): string {
  if (name.endsWith(".tar.gz")) return "application/gzip";
  if (name.endsWith(".zip")) return "application/zip";
  if (name.endsWith(".json")) return "application/json; charset=utf-8";
  if (name.endsWith(".txt")) return "text/plain; charset=utf-8";
  if (name.endsWith(".sh")) return "text/x-shellscript; charset=utf-8";
  if (name.endsWith(".ps1")) return "text/plain; charset=utf-8";
  return "application/octet-stream";
}
