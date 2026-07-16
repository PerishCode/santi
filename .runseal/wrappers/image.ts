//! `runseal :image <initial|terminal|faithful>` — the registry-integrity
//! gate on the never-reused versioned tag (CI-only; the mirror registries
//! are appliance closed loops). Requests go straight to the authority
//! hostname with proxies bypassed and redirects refused; the effective peer
//! is recorded as diagnostic evidence only. The initial phase records the
//! authority/selector/runner snapshot under RUNNER_TEMP; the terminal phase
//! re-verifies everything and requires the snapshot unchanged. Fail-closed;
//! certificate verification is never disabled.

import {
  agree,
  Answer,
  AUTHORITY_FILE,
  loadAuthority,
  selector,
  snapshot,
  Trace,
  unchanged,
  verify,
  WORKFLOW_FILE,
} from "@/lib/ci/image.ts";
import { repoRoot } from "@/lib/std/repo.ts";
import { join } from "@/lib/std/fs.ts";

async function client(url: string, accept: string): Promise<Answer> {
  const headers = await Deno.makeTempFile();
  const body = await Deno.makeTempFile();
  try {
    const output = await new Deno.Command("curl", {
      args: [
        "--silent",
        "--show-error",
        "--noproxy",
        "*",
        "--max-redirs",
        "0",
        "-H",
        `Accept: ${accept}`,
        "-D",
        headers,
        "-o",
        body,
        "-w",
        "%{remote_ip} %{http_code} %{num_redirects}",
        url,
      ],
      stdout: "piped",
      stderr: "piped",
    }).output();
    if (output.code !== 0) {
      throw new Error(
        `curl failed (${output.code}): ${new TextDecoder().decode(output.stderr).trim()}`,
      );
    }
    const [remote, status, redirects] = new TextDecoder().decode(output.stdout).trim().split(" ");
    const head = await Deno.readTextFile(headers);
    const digest = head.match(/^docker-content-digest:\s*(\S+)/im)?.[1];
    return {
      status: Number(status),
      remote,
      redirects: Number(redirects),
      digest,
      body: await Deno.readFile(body),
    };
  } finally {
    await Deno.remove(headers).catch(() => {});
    await Deno.remove(body).catch(() => {});
  }
}

function statePath(): string {
  return `${Deno.env.get("RUNNER_TEMP") ?? "/tmp"}/drift-gate.json`;
}

export async function main(argv: string[]): Promise<number> {
  const phase = argv[0] ?? "initial";
  const root = repoRoot();
  const text = await Deno.readTextFile(join(root, AUTHORITY_FILE));
  const authority = loadAuthority(text);
  const workflow = await Deno.readTextFile(join(root, WORKFLOW_FILE));
  const chosen = selector(workflow);
  console.log(`gate phase ${phase}`);
  const disagreement = agree(authority, chosen);
  if (disagreement.length > 0) {
    for (const line of disagreement) {
      console.error(line);
    }
    console.error(`registry-integrity gate FAILED (${phase}, selector)`);
    return 1;
  }
  const runner = Deno.env.get("RUNNER_NAME") ?? undefined;
  const { problems, evidence } = await verify(authority, runner, client);
  for (const line of evidence) {
    console.log(line);
  }
  if (problems.length > 0) {
    for (const line of problems) {
      console.error(line);
    }
    console.error(`registry-integrity gate FAILED (${phase})`);
    return 1;
  }
  const current = await snapshot(text, authority, chosen, runner as string);
  if (phase === "initial") {
    await Deno.writeTextFile(statePath(), `${JSON.stringify(current)}\n`);
  }
  if (phase === "terminal") {
    let initial: Trace;
    try {
      initial = JSON.parse(await Deno.readTextFile(statePath())) as Trace;
    } catch {
      console.error("terminal gate cannot read the initial snapshot record");
      console.error(`registry-integrity gate FAILED (${phase})`);
      return 1;
    }
    const moved = unchanged(initial, current);
    if (moved.length > 0) {
      for (const line of moved) {
        console.error(line);
      }
      console.error(`registry-integrity gate FAILED (${phase})`);
      return 1;
    }
    console.log(
      `snapshot unchanged since the initial gate (${initial.reference}, ${initial.runner})`,
    );
  }
  console.log(
    `registry-integrity gate: ok (${phase}; never-reused versioned tag verified — not content-addressed execution, not attestation)`,
  );
  return 0;
}

if (import.meta.main) {
  Deno.exit(await main(Deno.args));
}
