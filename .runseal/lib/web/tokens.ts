//! Token-consumption/binding checker (D6, Liberte 2026-07-15).
//!
//! Three directions, all mandatory: every token registered in `tokens.scss`
//! is consumed by some atom sheet; every token an atom sheet consumes is
//! registered; and `themes/glass.scss` binds every registered token exactly
//! once. An unused token is a squatter and fails the guard.

import { join } from "@/lib/std/fs.ts";

const TERRITORY = "web/src/components";
const REGISTRY = `${TERRITORY}/tokens.scss`;
const THEME = `${TERRITORY}/themes/glass.scss`;
const ORGANS = new Set(["tokens.scss", "media.scss"]);

export function registered(source: string): string[] {
  return [...source.matchAll(/@property --([\w-]+)/g)].map((match) => match[1]);
}

export function consumed(source: string): string[] {
  return [...source.matchAll(/var\(--([\w-]+)/g)].map((match) => match[1]);
}

export function bound(source: string): Map<string, number> {
  const counts = new Map<string, number>();
  for (const match of source.matchAll(/--([\w-]+):/g)) {
    counts.set(match[1], (counts.get(match[1]) ?? 0) + 1);
  }
  return counts;
}

export async function check(root: string): Promise<string[]> {
  const problems: string[] = [];
  const registry = registered(await Deno.readTextFile(join(root, REGISTRY)));
  const seats = new Set(registry);
  if (seats.size !== registry.length) {
    problems.push("tokens.scss registers a token twice");
  }

  const uses = new Set<string>();
  const walk = async (relative: string) => {
    for await (const entry of Deno.readDir(join(root, relative))) {
      const path = `${relative}/${entry.name}`;
      if (entry.isDirectory) {
        if (entry.name !== "themes") {
          await walk(path);
        }
        continue;
      }
      if (!entry.name.endsWith(".scss") || ORGANS.has(entry.name)) {
        continue;
      }
      for (const token of consumed(await Deno.readTextFile(join(root, path)))) {
        uses.add(token);
        if (!seats.has(token)) {
          problems.push(`${path}: consumes unregistered token --${token}`);
        }
      }
    }
  };
  await walk(TERRITORY);

  for (const token of registry) {
    if (!uses.has(token)) {
      problems.push(`tokens.scss: --${token} is registered but consumed nowhere (squatter)`);
    }
  }

  const bindings = bound(await Deno.readTextFile(join(root, THEME)));
  for (const token of registry) {
    const count = bindings.get(token) ?? 0;
    if (count !== 1) {
      problems.push(`${THEME}: --${token} bound ${count} times (exactly once required)`);
    }
  }
  for (const token of bindings.keys()) {
    if (!seats.has(token)) {
      problems.push(`${THEME}: binds unregistered token --${token}`);
    }
  }
  return problems;
}
