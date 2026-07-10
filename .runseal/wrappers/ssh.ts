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
  console.log("Usage: runseal :ssh <host> [--tty] [-- <remote-command>...]");
  console.log("");
  console.log("Reaches a santi-operated host through .local/ssh/config (gitignored;");
  console.log("seed .runseal/templates/ssh/config). The host must be declared by a Host line.");
  console.log("");
  console.log("With no remote command, :ssh opens an interactive shell and allocates a TTY");
  console.log("when stdin is a terminal. Use --tty to force TTY allocation for a remote command.");
}

function shellQuote(value: string): string {
  return `'${value.replaceAll("'", `'"'"'`)}'`;
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
let rest = args.slice(1);
const forceTty = rest[0] === "--tty";
if (forceTty) {
  rest = rest.slice(1);
}

if (!(await hostDeclared(`${root}/${SSH_CONFIG}`, host))) {
  console.error(
    `:ssh: host not declared in ${SSH_CONFIG}: ${host} (provision it from templates/ssh/config)`,
  );
  Deno.exit(1);
}

// cwd = repo root so the config's relative IdentityFile (.local/ssh/…) resolves.
const interactiveStdin = Deno.stdin.isTerminal();
const ttyArgs = forceTty || (rest.length === 0 && interactiveStdin) ? ["-tt"] : [];
let code: number;
if (rest.length === 0) {
  code = await run("ssh", [...ttyArgs, "-F", SSH_CONFIG, host], { cwd: root });
} else if (rest[0] === "--") {
  const command = rest.slice(1);
  if (command.length === 0) {
    console.error(":ssh: missing remote command after --");
    Deno.exit(2);
  }
  const remote = command.length === 1 ? command[0] : command.map(shellQuote).join(" ");
  code = await run("ssh", [...ttyArgs, "-F", SSH_CONFIG, host, remote], { cwd: root });
} else {
  console.error(":ssh: remote command must be separated with --");
  Deno.exit(2);
}
Deno.exit(code);
