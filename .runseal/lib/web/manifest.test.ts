//! Cross-language conformance vectors + normalization law for the canonical
//! tree digest (`vectors.json` is the shared authority; the rust embed check
//! reproduces the same digests in W-B).

import { treeDigest } from "@/lib/web/manifest.ts";

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

async function assertRejects(work: () => Promise<unknown>, needle: string): Promise<void> {
  try {
    await work();
  } catch (error) {
    const text = error instanceof Error ? error.message : String(error);
    assert(text.includes(needle), `error lacks "${needle}": ${text}`);
    return;
  }
  throw new Error("expected an error, none was thrown");
}

import vectors from "./vectors.json" with { type: "json" };

for (const vector of vectors.vectors) {
  Deno.test(`vector: ${vector.name}`, async () => {
    const entries = vector.files.map((file) => ({
      path: file.path,
      bytes: new TextEncoder().encode(file.text),
    }));
    assertEquals(await treeDigest(entries), vector.digest);
  });
}

Deno.test("duplicate paths are rejected", async () => {
  const entry = { path: "a.txt", bytes: new Uint8Array() };
  await assertRejects(() => treeDigest([entry, entry]), "duplicate path");
});

Deno.test("parent segments are rejected", async () => {
  await assertRejects(
    () => treeDigest([{ path: "a/../b.txt", bytes: new Uint8Array() }]),
    "unnormalized path segment",
  );
});

Deno.test("absolute paths are rejected", async () => {
  await assertRejects(
    () => treeDigest([{ path: "/etc/shadow", bytes: new Uint8Array() }]),
    "unnormalized path",
  );
});

Deno.test("backslashes are rejected", async () => {
  await assertRejects(
    () => treeDigest([{ path: "a\\b.txt", bytes: new Uint8Array() }]),
    "unnormalized path",
  );
});
