//! `runseal :image <initial|terminal|faithful>` — the address-bound
//! registry-drift gate (CI-only; the mirror registries are per-appliance
//! closed loops). Member selection comes from the runner-injected
//! /etc/hosts mapping of the registry hostname; every request is forced to
//! the selected address via `curl --resolve` (hostname preserved for TLS
//! verification, proxies bypassed, redirects refused, effective peer
//! verified). The initial phase records its selection under RUNNER_TEMP;
//! the terminal phase requires the selection unchanged. Fail-closed
//! throughout; certificate verification is never disabled.

import {
  agree,
  Answer,
  AUTHORITY_FILE,
  HOSTS_FILE,
  loadAuthority,
  mapping,
  member,
  Selection,
  selector,
  unchanged,
  verify,
  WORKFLOW_FILE,
} from "@/lib/ci/image.ts";
import { repoRoot } from "@/lib/std/repo.ts";
import { join } from "@/lib/std/fs.ts";

async function client(url: string, accept: string, address: string): Promise<Answer> {
  const host = new URL(url).hostname;
  const headers = await Deno.makeTempFile();
  const body = await Deno.makeTempFile();
  try {
    const output = await new Deno.Command("curl", {
      args: [
        "--silent",
        "--show-error",
        "--resolve",
        `${host}:443:${address}`,
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
  const authority = loadAuthority(await Deno.readTextFile(join(root, AUTHORITY_FILE)));
  const workflow = await Deno.readTextFile(join(root, WORKFLOW_FILE));
  const disagreement = agree(authority, selector(workflow));
  console.log(`gate phase ${phase}`);
  if (disagreement.length > 0) {
    for (const line of disagreement) {
      console.error(line);
    }
    console.error(`registry-drift gate FAILED (${phase}, selector)`);
    return 1;
  }
  const mapped = mapping(await Deno.readTextFile(HOSTS_FILE), authority.registry);
  if (mapped.problem !== undefined) {
    console.error(mapped.problem);
    console.error(`registry-drift gate FAILED (${phase}, pre-network)`);
    return 1;
  }
  const found = member(authority, mapped.address as string);
  if (found.problem !== undefined) {
    console.error(found.problem);
    console.error(`registry-drift gate FAILED (${phase}, pre-network)`);
    return 1;
  }
  const chosen: Selection = { label: found.label as string, address: mapped.address as string };
  const { problems, evidence } = await verify(authority, chosen, client);
  for (const line of evidence) {
    console.log(line);
  }
  if (problems.length > 0) {
    for (const line of problems) {
      console.error(line);
    }
    console.error(`registry-drift gate FAILED (${phase})`);
    return 1;
  }
  if (phase === "initial") {
    await Deno.writeTextFile(statePath(), `${JSON.stringify(chosen)}\n`);
  }
  if (phase === "terminal") {
    let initial: Selection;
    try {
      initial = JSON.parse(await Deno.readTextFile(statePath())) as Selection;
    } catch {
      console.error("terminal gate cannot read the initial selection record");
      console.error(`registry-drift gate FAILED (${phase})`);
      return 1;
    }
    const moved = unchanged(initial, chosen);
    if (moved.length > 0) {
      for (const line of moved) {
        console.error(line);
      }
      console.error(`registry-drift gate FAILED (${phase})`);
      return 1;
    }
    console.log(`selection unchanged since the initial gate (${initial.label}@${initial.address})`);
  }
  console.log(
    `registry-drift gate: ok (${phase}; address-bound drift detection only — not a pinned container, not machine attestation)`,
  );
  return 0;
}

if (import.meta.main) {
  Deno.exit(await main(Deno.args));
}
