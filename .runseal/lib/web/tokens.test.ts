//! Three-direction token law on synthetic organ sources.

import { bound, consumed, registered } from "@/lib/web/tokens.ts";

function assertEquals(actual: unknown, expected: unknown, message = "values differ"): void {
  const left = JSON.stringify(actual);
  const right = JSON.stringify(expected);
  if (left !== right) {
    throw new Error(`${message}: actual ${left}, expected ${right}`);
  }
}

Deno.test("registered parses @property seats", () => {
  const source = `@property --ink {\n\tsyntax: "<color>";\n}\n@property --space-2 {\n}\n`;
  assertEquals(registered(source), ["ink", "space-2"]);
});

Deno.test("consumed parses var() uses including fallbacks", () => {
  const source = `.a { color: var(--ink); padding: var(--space-2) 0; }`;
  assertEquals(consumed(source), ["ink", "space-2"]);
});

Deno.test("bound counts theme bindings", () => {
  const source = `:root {\n\t--ink: #fff;\n\t--ink: #000;\n\t--space-2: 1rem;\n}\n`;
  const counts = bound(source);
  assertEquals(counts.get("ink"), 2);
  assertEquals(counts.get("space-2"), 1);
});
