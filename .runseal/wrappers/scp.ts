//! `runseal :scp <source> <destination>`
//!
//! Copy a file to or from a declared santi-operated host through the repo-local
//! SSH config. Remote operands use scp's `<host>:<path>` form.

import { run } from "@/lib/std/cmd.ts";
import { repoRoot } from "@/lib/std/repo.ts";
import { hostDeclared } from "@/lib/sshconfig.ts";

const SSH_CONFIG = ".local/ssh/config";

function usage(): void {
  console.log("Usage: runseal :scp <source> <destination>");
  console.log("");
  console.log("Copies through .local/ssh/config. At least one operand must use");
  console.log("the <declared-host>:<path> form; arbitrary scp options are unsupported.");
}

function remoteHost(operand: string): string | undefined {
  if (/^[A-Za-z]:[\\/]/.test(operand)) return undefined;
  const separator = operand.indexOf(":");
  if (separator <= 0) return undefined;
  return operand.slice(0, separator);
}

const args = [...Deno.args];
if (args.length === 1 && ["-h", "--help", "help"].includes(args[0])) {
  usage();
  Deno.exit(0);
}
if (args.length !== 2 || args.some((operand) => operand.startsWith("-"))) {
  usage();
  Deno.exit(2);
}

const root = repoRoot();
const hosts = args.map(remoteHost).filter((host): host is string => host !== undefined);
if (hosts.length === 0) {
  console.error(":scp: at least one operand must name a remote host");
  Deno.exit(2);
}
for (const host of new Set(hosts)) {
  if (!(await hostDeclared(`${root}/${SSH_CONFIG}`, host))) {
    console.error(`:scp: host not declared in ${SSH_CONFIG}: ${host}`);
    Deno.exit(1);
  }
}

const code = await run("scp", ["-F", SSH_CONFIG, "--", ...args], { cwd: root });
Deno.exit(code);
