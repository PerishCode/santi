//! Strict release smoke: install the published build from the public URL via
//! the distribution manager, then verify `santi --help` and a live
//! `santi service` /health, then uninstall.

import { run } from "@/lib/std/cmd.ts";
import { exists, join } from "@/lib/std/fs.ts";
import { fail, publicUrl, required } from "@/lib/release/env.ts";

const HEALTH_PORT = 47193;
const HEALTH_TIMEOUT_MS = 20_000;

export async function smoke(repo: string): Promise<void> {
  const channel = required("RELEASE_CHANNEL");
  const version = required("RELEASE_VERSION");
  const isWindows = Deno.build.os === "windows";
  const binName = isWindows ? "santi.exe" : "santi";

  const tmp = await Deno.makeTempDir({ prefix: "santi-smoke-" });
  const installRoot = join(tmp, "install");
  const binDir = join(tmp, "bin");
  Deno.mkdirSync(installRoot, { recursive: true });
  Deno.mkdirSync(binDir, { recursive: true });

  const managerEnv: Record<string, string> = {
    SANTI_RELEASES_PUBLIC_URL: publicUrl(),
    SANTI_INSTALL_ROOT: installRoot,
    SANTI_LOCAL_BIN_DIR: binDir,
  };

  await manage(repo, isWindows, [
    "install",
    "--channel",
    channel,
    "--version",
    version,
    "--retain=false",
  ], managerEnv);

  const bin = join(binDir, binName);
  if (!exists(bin)) fail(`install did not place ${bin}`);
  if (await run(bin, ["--help"]) !== 0) fail("santi --help failed");

  await serviceHealth(tmp, bin);

  await manage(repo, isWindows, ["uninstall", "--version", version], managerEnv);
  if (exists(join(installRoot, normalizeVersion(version)))) {
    fail(`uninstall left ${join(installRoot, version)}`);
  }
  console.log(
    `[release] smoke ok: ${version} installed, --help + service health passed, uninstalled`,
  );
}

function normalizeVersion(value: string): string {
  return `v${value.replace(/^v/, "")}`;
}

async function manage(
  repo: string,
  isWindows: boolean,
  args: string[],
  env: Record<string, string>,
): Promise<void> {
  const code = isWindows
    ? await run("pwsh", ["-NoProfile", "-File", join(repo, "manage.ps1"), ...args], { env })
    : await run("sh", [join(repo, "manage.sh"), ...args], { env });
  if (code !== 0) fail(`manage ${args[0]} failed`);
}

async function serviceHealth(tmp: string, bin: string): Promise<void> {
  const config = join(tmp, "smoke.santi.toml");
  Deno.writeTextFileSync(
    config,
    [
      'provider = "smoke"',
      "",
      "[providers.smoke]",
      'kind = "chat_completions"',
      'api_key = "smoke"',
      'model = "smoke"',
      'base_url = "http://127.0.0.1:1"',
      "",
    ].join("\n"),
  );

  const child = new Deno.Command(bin, {
    args: ["service", "serve"],
    cwd: tmp, // avoid picking up the repo's gitignored .env
    env: {
      SANTI_CONFIG: config,
      SANTI_PROVIDER: "smoke",
      SANTI_DB: join(tmp, "smoke.sqlite"),
      SANTI_HOST: "127.0.0.1",
      SANTI_PORT: String(HEALTH_PORT),
      SANTI_RUNTIME_ROOT: join(tmp, "runtime"),
      SANTI_EXECUTION_ROOT: join(tmp, "execution"),
    },
    stdin: "null",
    stdout: "piped",
    stderr: "piped",
  }).spawn();

  try {
    if (!(await waitHealth(`http://127.0.0.1:${HEALTH_PORT}/api/v1/health`, HEALTH_TIMEOUT_MS))) {
      fail("santi service /health did not become ready");
    }
    console.log("[release] service health ok");
  } finally {
    try {
      child.kill("SIGTERM");
    } catch {
      // already gone
    }
    await child.status;
  }
}

async function waitHealth(url: string, timeoutMs: number): Promise<boolean> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const response = await fetch(url);
      const text = await response.text();
      if (response.ok) {
        try {
          if ((JSON.parse(text) as { ok?: boolean }).ok === true) return true;
        } catch {
          // not json yet
        }
      }
    } catch {
      // not accepting connections yet
    }
    await new Promise((resolve) => setTimeout(resolve, 400));
  }
  return false;
}
