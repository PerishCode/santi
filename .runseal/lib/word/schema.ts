//! OpenAPI-contract wire-type evidence (W3 slice 1, Liberte 2026-07-14).
//!
//! An OpenAPI component name is an externally observable schema identifier.
//! A PascalCase Rust type whose spelling exactly matches a component, with a
//! unique workspace declaration, carries an identity binding to the wire and
//! classifies as wire wherever it is referenced. Membership is mechanically
//! derived from the exported contract — never hand-curated — and verified
//! fresh on every run so the evidence file cannot rot into an allowlist.

import { capture } from "@/lib/std/cmd.ts";
import { join } from "@/lib/std/fs.ts";

export interface Evidence {
  schema: string;
  digest: string;
  components: string[];
}

export const EVIDENCE_FILE = ".runseal/wire-schema.json";

export interface Contract {
  digest: string;
  components: string[];
}

/** Freshly export the OpenAPI document and derive digest + component list. */
export async function exportContract(root: string): Promise<Contract> {
  const result = await capture(
    "cargo",
    ["run", "-q", "-p", "santi", "--", "service", "export-openapi"],
    { cwd: root },
  );
  if (result.code !== 0) {
    throw new Error(
      `export-openapi failed (${result.code}): ${result.stderr.trim() || result.stdout.trim()}`,
    );
  }
  const text = result.stdout.replace(/\n$/, "");
  const digest = await sha256(text);
  const document = JSON.parse(text) as {
    components?: { schemas?: Record<string, unknown> };
  };
  const schemas = document.components?.schemas;
  if (!schemas || typeof schemas !== "object") {
    throw new Error("export-openapi: document has no components.schemas");
  }
  return { digest, components: Object.keys(schemas).sort() };
}

export async function loadEvidence(root: string): Promise<Evidence> {
  const text = await Deno.readTextFile(join(root, EVIDENCE_FILE));
  return JSON.parse(text) as Evidence;
}

export async function writeEvidence(root: string, contract: Contract): Promise<void> {
  const evidence: Evidence = {
    schema: "santi.wire_schema.v1",
    digest: contract.digest,
    components: contract.components,
  };
  const path = join(root, EVIDENCE_FILE);
  const tmp = `${path}.tmp`;
  await Deno.writeTextFile(tmp, `${JSON.stringify(evidence, null, 2)}\n`);
  await Deno.rename(tmp, path);
}

/** Fail closed when the checked-in evidence does not match a fresh contract. */
export function assertFresh(evidence: Evidence, contract: Contract): void {
  const problems = verifyEvidence(evidence, contract);
  if (problems.length > 0) {
    throw new Error(problems.join("\n"));
  }
}

/** Compare checked-in evidence against the freshly exported contract. */
export function verifyEvidence(evidence: Evidence, contract: Contract): string[] {
  const problems: string[] = [];
  if (evidence.digest !== contract.digest) {
    problems.push(
      `${EVIDENCE_FILE} is stale: recorded contract digest ${evidence.digest} != fresh ${contract.digest}`,
    );
  }
  if (evidence.components.join("\n") !== contract.components.join("\n")) {
    problems.push(
      `${EVIDENCE_FILE} is stale: recorded component list differs from the fresh contract`,
    );
  }
  if (problems.length > 0) {
    problems.push(
      "regenerate explicitly with: runseal :word-debt --sync-wire-schema (the diff is subject to review)",
    );
  }
  return problems;
}

/**
 * Count Rust type declarations of `token` across the workspace. The identity
 * binding requires exactly one owner; anything else declines the override so
 * same-spelled strangers can never hide behind a schema name.
 */
export async function declarationCount(root: string, token: string): Promise<number> {
  const result = await capture(
    "grep",
    ["-rE", `\\b(struct|enum|type)\\s+${token}\\b`, "crates", "--include=*.rs", "-h"],
    { cwd: root },
  );
  if (result.code === 1) {
    return 0;
  }
  if (result.code !== 0) {
    throw new Error(`declaration scan for ${token} failed: ${result.stderr.trim()}`);
  }
  return result.stdout.split("\n").filter((line) => line.trim().length > 0).length;
}

async function sha256(text: string): Promise<string> {
  const bytes = new TextEncoder().encode(text);
  const hash = await crypto.subtle.digest("SHA-256", bytes);
  return Array.from(new Uint8Array(hash))
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("");
}
