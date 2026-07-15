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
  properties: Record<string, string[]>;
}

export const EVIDENCE_FILE = ".runseal/wire-schema.json";

export interface Contract {
  digest: string;
  components: string[];
  properties: Record<string, string[]>;
  schemas: Record<string, Record<string, unknown>>;
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
  const components = Object.keys(schemas).sort();
  const properties: Record<string, string[]> = {};
  for (const name of components) {
    properties[name] = propertyNames(schemas[name]);
  }
  return {
    digest,
    components,
    properties,
    schemas: schemas as Record<string, Record<string, unknown>>,
  };
}

function propertyNames(schema: unknown): string[] {
  const names = new Set<string>();
  const visit = (node: unknown) => {
    if (!node || typeof node !== "object") {
      return;
    }
    const record = node as Record<string, unknown>;
    const own = record.properties;
    if (own && typeof own === "object") {
      for (const key of Object.keys(own)) {
        names.add(key);
      }
    }
    for (const branch of ["oneOf", "allOf", "anyOf"]) {
      const variants = record[branch];
      if (Array.isArray(variants)) {
        variants.forEach(visit);
      }
    }
  };
  visit(schema);
  return [...names].sort();
}

export async function loadEvidence(root: string): Promise<Evidence> {
  const text = await Deno.readTextFile(join(root, EVIDENCE_FILE));
  return JSON.parse(text) as Evidence;
}

export async function writeEvidence(root: string, contract: Contract): Promise<void> {
  const evidence: Evidence = {
    schema: "santi.wire_schema.v2",
    digest: contract.digest,
    components: contract.components,
    properties: contract.properties,
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
  if (JSON.stringify(evidence.properties ?? null) !== JSON.stringify(contract.properties)) {
    problems.push(
      `${EVIDENCE_FILE} is stale: recorded property lists differ from the fresh contract`,
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

export interface FieldVerdict {
  lift: boolean;
  component?: string;
  reason: string;
}

/**
 * Live owner-aware binding (W3 slice 2): the finding at `line` lifts only if
 * a recognized component's brace-balanced declaration span contains it, the
 * token is that component's contract property, and the line is an unrenamed
 * `pub` field declaration. Everything else stays internal.
 */
export function fieldBinding(
  source: string,
  components: string[],
  properties: Record<string, string[]>,
  token: string,
  line: number,
): FieldVerdict {
  const containing = componentSpans(source, components).filter(
    (span) => span.start <= line && line <= span.end,
  );
  if (containing.length === 0) {
    return { lift: false, reason: "outside any component declaration span" };
  }
  if (containing.length > 1) {
    return {
      lift: false,
      component: containing[0].component,
      reason: `line ${line} sits in ${containing.length} overlapping owner spans (ambiguous)`,
    };
  }
  const owner = containing[0];
  if (!(properties[owner.component] ?? []).includes(token)) {
    return {
      lift: false,
      component: owner.component,
      reason: `${token} is not a contract property of ${owner.component}`,
    };
  }
  const lines = source.split("\n");
  const text = lines[line - 1] ?? "";
  if (!new RegExp(`pub ${token}\\s*:`).test(text)) {
    return {
      lift: false,
      component: owner.component,
      reason: `line ${line} is not a pub field declaration of ${token}`,
    };
  }
  for (let index = line - 2; index >= 0; index -= 1) {
    const above = lines[index].trim();
    if (!above.startsWith("#[")) {
      break;
    }
    if (above.includes("rename")) {
      return {
        lift: false,
        component: owner.component,
        reason:
          `${owner.component}.${token} carries a serde rename; Rust spelling is not the wire spelling`,
      };
    }
  }
  return {
    lift: true,
    component: owner.component,
    reason: `${owner.component}.${token} = contract property (identity binding)`,
  };
}

function componentSpans(
  source: string,
  components: string[],
): Array<{ component: string; start: number; end: number }> {
  const spans: Array<{ component: string; start: number; end: number }> = [];
  const lines = source.split("\n");
  const wanted = new Set(components);
  for (let index = 0; index < lines.length; index += 1) {
    const declared = lines[index].match(/\b(?:struct|enum)\s+(\w+)/);
    if (!declared || !wanted.has(declared[1])) {
      continue;
    }
    let depth = 0;
    let opened = false;
    for (let scan = index; scan < lines.length; scan += 1) {
      for (const char of lines[scan]) {
        if (char === "{") {
          depth += 1;
          opened = true;
        } else if (char === "}") {
          depth -= 1;
        }
      }
      if (opened && depth === 0) {
        spans.push({ component: declared[1], start: index + 1, end: scan + 1 });
        break;
      }
    }
  }
  return spans;
}
