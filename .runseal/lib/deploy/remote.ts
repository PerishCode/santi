//! SSH transport for the host-side deploy program.

import { join } from "@/lib/std/fs.ts";
import { repoRoot } from "@/lib/std/repo.ts";

const HOST = "hk-03.zxiyun";
const SSH_CONFIG = ".local/ssh/config";
const SCRIPT = ".runseal/ops/host/40-santi-deploy.sh";
const VERSION = /^v[0-9]+\.[0-9]+\.[0-9]+-beta\.[1-9][0-9]*$/;

export async function runDeployRemote(version: string): Promise<number> {
  if (!VERSION.test(version)) {
    throw new Error(`unsafe deploy version: ${JSON.stringify(version)}`);
  }
  const root = repoRoot();
  const program = await Deno.readFile(join(root, SCRIPT));
  const child = new Deno.Command("ssh", {
    args: ["-F", SSH_CONFIG, HOST, `bash -s -- ${version}`],
    cwd: root,
    stdin: "piped",
    stdout: "inherit",
    stderr: "inherit",
  }).spawn();
  const writer = child.stdin.getWriter();
  await writer.write(program);
  await writer.close();
  return (await child.status).code;
}
