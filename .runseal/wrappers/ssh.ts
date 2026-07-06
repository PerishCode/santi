//! `runseal :ssh <host> [-- <remote-command>...]`
//!
//! Reach a santi-operated host through the repo-local `.local/ssh/config`
//! (gitignored; seed at `.runseal/templates/ssh/config`). The host must be
//! declared by a `Host` line in that config; arbitrary -F/-i overrides are
//! unsupported. Internalized into santi so its remote ops need no infra clone.

import { run } from "@/lib/std/cmd.ts";
import { repoRoot } from "@/lib/std/repo.ts";
import { hostDeclared } from "@/lib/sshconfig.ts";

const SSH_CONFIG = ".local/ssh/config"; // relative to the repo root

function usage(): void {
  console.log("Usage: runseal :ssh <host> [-- <remote-command>...]");
  console.log("");
  console.log("Reaches a santi-operated host through .local/ssh/config (gitignored;");
  console.log("seed .runseal/templates/ssh/config). The host must be declared by a Host line.");
}

const args = [...Deno.args];
if (args.length === 0) {
  usage();
  Deno.exit(2);
}
if (["-h", "--help", "help"].includes(args[0])) {
  usage();
  Deno.exit(0);
}

const root = repoRoot();
const host = args[0];
const rest = args.slice(1);

if (!(await hostDeclared(`${root}/${SSH_CONFIG}`, host))) {
  console.error(
    `:ssh: host not declared in ${SSH_CONFIG}: ${host} (provision it from templates/ssh/config)`,
  );
  Deno.exit(1);
}

// cwd = repo root so the config's relative IdentityFile (.local/ssh/…) resolves.
let code: number;
if (rest.length === 0) {
  code = await run("ssh", ["-F", SSH_CONFIG, host], { cwd: root });
} else if (rest[0] === "--") {
  code = await run("ssh", ["-F", SSH_CONFIG, host, ...rest.slice(1)], { cwd: root });
} else {
  console.error(":ssh: remote command must be separated with --");
  Deno.exit(2);
}
Deno.exit(code);
