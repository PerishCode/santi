//! C4 gate regression tests (Liberte 2026-07-14): synthetic findings run
//! through the SAME parse/applyWireEvidence/ratchet path the command uses,
//! and the asserted value is the command's exit code itself.

import { applyWireEvidence, Baseline, parse, ratchet } from "@/lib/word/report.ts";
import { classify } from "@/lib/word/tracks.ts";

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

const NO_PROPS: Record<string, string[]> = {};

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

Deno.test("grouped durable store seats stay wire", () => {
  for (
    const path of [
      "crates/santi-core/src/store/ledger/db.rs",
      "crates/santi-core/src/store/ledger/rows.rs",
      "crates/santi-core/src/store/ledger/souls.rs",
    ]
  ) {
    assertEquals(classify(path, "strand_id"), "wire");
  }
  assertEquals(
    classify("crates/santi-core/src/store/ledger/turn.rs", "strand_id"),
    "internal",
  );
});

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
    schema: "santi.wire_schema.v2",
    digest: "aaaa",
    components: ["StrandMaterial"],
    properties: { StrandMaterial: ["strand_id"] },
  };
  assertThrows(
    () =>
      assertFresh(evidence, {
        digest: "bbbb",
        components: ["StrandMaterial"],
        properties: { StrandMaterial: ["strand_id"] },
        schemas: {},
      }),
    Error,
    "stale",
  );
  assertThrows(
    () =>
      assertFresh(evidence, {
        digest: "aaaa",
        components: ["Other"],
        properties: { StrandMaterial: ["strand_id"] },
        schemas: {},
      }),
    Error,
    "stale",
  );
  assertThrows(
    () =>
      assertFresh(evidence, {
        digest: "aaaa",
        components: ["StrandMaterial"],
        properties: { StrandMaterial: ["strand_id", "other"] },
        schemas: {},
      }),
    Error,
    "property lists differ",
  );
});

Deno.test("ambiguous schema spelling declines the wire override", async () => {
  const raw = [
    "crates/santi-core/src/service.rs:10:5 debt StrandMaterial",
    summary(0, 1),
  ].join("\n");
  const report = parse(raw);
  assertEquals(report.tracks.core.occurrences, 1);
  const notes = await applyWireEvidence(
    ".",
    report,
    { schema: "v2", digest: "d", components: ["StrandMaterial"], properties: NO_PROPS },
    () => Promise.resolve(2),
  );
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
  const notes = await applyWireEvidence(
    ".",
    report,
    { schema: "v2", digest: "d", components: ["StrandMaterial"], properties: NO_PROPS },
    () => Promise.resolve(1),
  );
  assertEquals(report.tracks.core.occurrences, 0);
  assertEquals(report.tracks.wire.occurrences, 1);
  assert(notes.some((m) => m.includes("core -> wire")));
  assertEquals(ratchet(report, BASELINE).code, 0);
});

const FIELD_SOURCE = `#[derive(Debug, Serialize)]
pub struct CompactThing {
    #[serde(default)]
    pub pre_estimate: Option<i64>,
    #[serde(rename = "sealed_name")]
    pub inner_estimate: Option<i64>,
}

fn helper() {
    let lone_estimate = 1;
}
`;

const FIELD_EVIDENCE: Evidence = {
  schema: "santi.wire_schema.v2",
  digest: "d",
  components: ["CompactThing"],
  properties: { CompactThing: ["pre_estimate", "inner_estimate", "lone_estimate"] },
};

function fieldReport(rows: string[]) {
  return parse([...rows, summary(0, rows.length)].join("\n"));
}

const readFixture = () => Promise.resolve(FIELD_SOURCE);

Deno.test("contract property field lifts internal to wire", async () => {
  const report = fieldReport(["crates/santi-core/src/model/fixture.rs:4:5 debt pre_estimate"]);
  assertEquals(report.tracks.internal.occurrences, 1);
  const notes = await applyWireEvidence(
    ".",
    report,
    FIELD_EVIDENCE,
    () => Promise.resolve(1),
    readFixture,
  );
  assertEquals(report.tracks.internal.occurrences, 0);
  assertEquals(report.tracks.wire.occurrences, 1);
  assert(notes.some((m) => m.includes("CompactThing.pre_estimate = contract property")));
  assertEquals(ratchet(report, BASELINE).code, 0);
});

Deno.test("same-suffix local outside the span stays internal", async () => {
  const report = fieldReport(["crates/santi-core/src/model/fixture.rs:10:9 debt lone_estimate"]);
  const notes = await applyWireEvidence(
    ".",
    report,
    FIELD_EVIDENCE,
    () => Promise.resolve(1),
    readFixture,
  );
  assertEquals(report.tracks.internal.occurrences, 1, "local must stay internal");
  assertEquals(notes.length, 0, "outside-span rows decline silently");
});

Deno.test("serde-renamed field never lifts", async () => {
  const report = fieldReport(["crates/santi-core/src/model/fixture.rs:6:5 debt inner_estimate"]);
  const notes = await applyWireEvidence(
    ".",
    report,
    FIELD_EVIDENCE,
    () => Promise.resolve(1),
    readFixture,
  );
  assertEquals(report.tracks.internal.occurrences, 1);
  assert(notes.some((m) => m.includes("serde rename") && m.includes("stays internal")));
});

Deno.test("mixed field and local group declines loudly", async () => {
  const report = fieldReport([
    "crates/santi-core/src/model/fixture.rs:4:5 debt pre_estimate",
    "crates/santi-core/src/model/fixture.rs:20:9 debt pre_estimate",
  ]);
  const notes = await applyWireEvidence(
    ".",
    report,
    FIELD_EVIDENCE,
    () => Promise.resolve(1),
    readFixture,
  );
  assertEquals(
    report.tracks.internal.occurrences,
    2,
    "a failing row keeps the whole group internal",
  );
  assert(notes.some((m) => m.includes("group stays internal")), "decline must be loud");
  assert(
    notes.some((m) => m.includes("fixture.rs:4") && m.includes("failed verification")),
    "the passing twin must be named too",
  );
});

const TWIN_SOURCE = `pub struct First {
    pub shared_estimate: i64,
}

pub struct Second {
    pub shared_estimate: i64,
}
`;

Deno.test("distinct-owner field twins lift as a group", async () => {
  const report = fieldReport([
    "crates/santi-core/src/model/fixture.rs:2:5 debt shared_estimate",
    "crates/santi-core/src/model/fixture.rs:6:5 debt shared_estimate",
  ]);
  const evidence: Evidence = {
    schema: "santi.wire_schema.v2",
    digest: "d",
    components: ["First", "Second"],
    properties: { First: ["shared_estimate"], Second: ["shared_estimate"] },
  };
  const notes = await applyWireEvidence(
    ".",
    report,
    evidence,
    () => Promise.resolve(1),
    () => Promise.resolve(TWIN_SOURCE),
  );
  assertEquals(report.tracks.internal.occurrences, 0, "both verified twins must lift");
  assertEquals(report.tracks.wire.occurrences, 2);
  assertEquals(notes.filter((m) => m.includes("identity binding")).length, 2);
});

import { lawLines } from "@/lib/word/report.ts";

Deno.test("law summary parses both singular and plural spellings", () => {
  const old = "0 faults, 0 blindspots, 2124 debt\nby law: word=2124";
  const now = "0 faults, 0 blindspots, 2124 debts\nby law: word=2124";
  assertEquals(lawLines(old).length, 2);
  assertEquals(lawLines(now).length, 2);
  assertEquals(lawLines("noise\nnot a summary").length, 0);
});
