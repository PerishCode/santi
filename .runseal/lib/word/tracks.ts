//! Word-debt tracks after the Liberte consult (2026-07-14).
//!
//! Mechanical scan exposes compound identifiers. Tracks decide which compounds
//! count as unexplained *semantic* debt versus protocol dialect, test prose, or
//! ordinary internal combinations. See `.task/MAIN.md` and Liberte's ruling:
//! classify first, ratchet second; zero applies only to re-defined core debt.

export type Track = "wire" | "test" | "core" | "internal";

export const TRACKS: readonly Track[] = ["wire", "test", "core", "internal"];

/**
 * Compatibility-boundary paths (operator 2026-07-14: full boundary is wire).
 * Includes HTTP/serde models, provider adaptors, CLI/env surface, and durable
 * SQLite schema / projection seats — not ordinary internal orchestration code.
 */
const WIRE_PATH =
  /(?:^|\/)(?:model(?:\/|\.rs$)|server\/(?:openapi|routes|sse|im|effects|errors|error\.rs$)|webhook(?:\/|\.rs$)|provider\.rs$|openai(?:\/|\.rs$)|chat\/completions(?:\/|\.rs$)|cli\.rs$|config\.rs$|ops\.rs$|upgrade(?:\/|\.rs$)|object\/store\.rs$|santi-error\/|store\/(?:db(?:\/|\.rs$)|schema\.rs$|rows\.rs$)|store\.rs$)/;

/** Protocol field dialect: multi-word spellings that encode wire/schema shape. */
const FIELD_DIALECT =
  /_(?:id|at|bytes|path|url|key|seq|root|count|mode|type|ref|text|name|raw|json|state|kind|source|status|label|version|prefix|index|size|limit|timeout|effort|summary|budget|rounds|calls|output|input|memory|strand|soul|turn|message|incident|receipt|effect|compact|delivery|participant|provider|response|request)$/;

/** Wire-facing type suffixes kept on the protocol boundary. */
const WIRE_TYPE = /(?:Request|Response|Params|Query|Report)$/;

export function classify(path: string, token: string): Track {
  if (path.includes("/tests/")) {
    return "test";
  }

  if (isScreaming(token)) {
    return WIRE_PATH.test(path) ? "wire" : "internal";
  }

  if (/^[A-Z]/.test(token)) {
    // Liberte: public ToSchema / compatibility-boundary types seat as wire.
    // Heuristic: Request/Response/… suffixes or types defined on wire paths
    // (model, HTTP, provider, CLI/config, durable store seats).
    if (WIRE_TYPE.test(token) || WIRE_PATH.test(path)) {
      return "wire";
    }
    return "core";
  }

  if (FIELD_DIALECT.test(token) || token.endsWith("_id") || token.endsWith("_at")) {
    return WIRE_PATH.test(path) ? "wire" : "internal";
  }

  return "internal";
}

function isScreaming(token: string): boolean {
  return token.includes("_") && token === token.toUpperCase() && /[A-Z]/.test(token);
}
