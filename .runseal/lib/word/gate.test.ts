//! C4 gate regression tests (Liberte 2026-07-14): synthetic findings run
//! through the SAME parse/applyWireEvidence/ratchet path the command uses,
//! and the asserted value is the command's exit code itself.

import { applyWireEvidence, Baseline, parse, ratchet } from "@/lib/word/report.ts";

function assert(condition: boolean, message = "assertion failed"): void {
  if (!condition) {
    throw new Error(message);
  }
}

function assertEquals(actual: unknown, expected: unknown, message = "values differ"): void {
  if (actual !== expected) {
    throw new Error(`${message}: actual ${actual}, expected ${expected}`);
  }
}

function assertThrows(callback: () => unknown, _kind: ErrorConstructor, needle: string): void {
  try {
    callback();
  } catch (error) {
    const text = error instanceof Error ? error.message : String(error);
    assert(text.includes(needle), `error lacks "${needle}": ${text}`);
    return;
  }
  throw new Error("expected an error, none was thrown");
}
import { assertFresh, Evidence } from "@/lib/word/schema.ts";

const BASELINE: Baseline = {
  schema: "santi.word_debt.v1",
  note: "test",
  total: 2059,
  tracks: {
    wire: { occurrences: 713, unique: 512 },
    test: { occurrences: 487, unique: 400 },
    internal: { occurrences: 859, unique: 681 },
  },
};

function summary(faults: number, debt: number): string {
  return `${faults} faults, 0 blindspots, ${debt} debt`;
}

Deno.test("one synthetic core occurrence fails the gate", () => {
  const raw = [
    "crates/santi-core/src/service.rs:10:5 debt SyntheticCoreThing",
    summary(0, 1),
  ].join("\n");
  const report = parse(raw);
  assertEquals(report.tracks.core.occurrences, 1);
  const result = ratchet(report, BASELINE);
  assert(result.code !== 0, "core > 0 must produce a failing exit code");
  assert(
    result.messages.some((m) =>
      m.includes("SyntheticCoreThing at crates/santi-core/src/service.rs:10")
    ),
    "diagnostics must name token and location",
  );
});

Deno.test("zero core passes the gate", () => {
  const raw = [
    "crates/santi-core/src/service.rs:10:5 debt plain_local",
    summary(0, 1),
  ].join("\n");
  const report = parse(raw);
  assertEquals(report.tracks.core.occurrences, 0);
  const result = ratchet(report, BASELINE);
  assertEquals(result.code, 0);
});

Deno.test("internal rise warns loudly but does not block", () => {
  const rows = Array.from(
    { length: BASELINE.tracks.internal.occurrences + 3 },
    (_, i) => `crates/santi-core/src/service.rs:${i + 1}:5 debt plain_local`,
  );
  const report = parse([...rows, summary(0, rows.length)].join("\n"));
  const result = ratchet(report, BASELINE);
  assertEquals(result.code, 0, "internal-only rise must stay non-blocking");
  assert(
    result.messages.some((m) => m.includes("internal track ROSE") && m.includes("+3")),
    "warning must name the positive delta",
  );
});

Deno.test("stale wire-schema evidence fails closed", () => {
  const evidence: Evidence = {
    schema: "santi.wire_schema.v1",
    digest: "aaaa",
    components: ["StrandMaterial"],
  };
  assertThrows(
    () => assertFresh(evidence, { digest: "bbbb", components: ["StrandMaterial"] }),
    Error,
    "stale",
  );
  assertThrows(
    () => assertFresh(evidence, { digest: "aaaa", components: ["Other"] }),
    Error,
    "stale",
  );
});

Deno.test("ambiguous schema spelling declines the wire override", async () => {
  const raw = [
    "crates/santi-core/src/service.rs:10:5 debt StrandMaterial",
    summary(0, 1),
  ].join("\n");
  const report = parse(raw);
  assertEquals(report.tracks.core.occurrences, 1);
  const notes = await applyWireEvidence(".", report, ["StrandMaterial"], () => Promise.resolve(2));
  assertEquals(report.tracks.core.occurrences, 1, "two declarations must stay core");
  assert(notes.some((m) => m.includes("declined")));
  const result = ratchet(report, BASELINE);
  assert(result.code !== 0, "declined override must surface as a gate failure");
});

Deno.test("unique schema spelling lifts core to wire", async () => {
  const raw = [
    "crates/santi-core/src/service.rs:10:5 debt StrandMaterial",
    summary(0, 1),
  ].join("\n");
  const report = parse(raw);
  const notes = await applyWireEvidence(".", report, ["StrandMaterial"], () => Promise.resolve(1));
  assertEquals(report.tracks.core.occurrences, 0);
  assertEquals(report.tracks.wire.occurrences, 1);
  assert(notes.some((m) => m.includes("core -> wire")));
  assertEquals(ratchet(report, BASELINE).code, 0);
});
