//! SSH transport for the host-side recovery program. The recovery logic is
//! streamed for each invocation so rollback never depends on the candidate
//! Santi binary or an on-host script installation.

import { join } from "@/lib/std/fs.ts";
import { repoRoot } from "@/lib/std/repo.ts";
import { remoteCommand } from "@perish/sealkit/operator";

const HOST = "hk-03.zxiyun";
const SSH_CONFIG = ".local/ssh/config";
const SCRIPT = ".runseal/ops/host/30-santi-recovery.sh";
export function recoveryRemoteCommand(argv: string[]): string {
  return remoteCommand(["bash", "-s", "--"], argv);
}

export async function runRecoveryRemote(argv: string[]): Promise<number> {
  const root = repoRoot();
  const program = await Deno.readFile(join(root, SCRIPT));
  const child = new Deno.Command("ssh", {
    args: ["-F", SSH_CONFIG, HOST, recoveryRemoteCommand(argv)],
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
