//! Enforce single-word Rust test entrypoint leaves.

import { join } from "@/lib/std/fs.ts";

export interface LeafFault {
  path: string;
  line: number;
  name: string;
}

const TEST_ATTRIBUTE = /^\s*#\[\s*(?:tokio::)?test(?:\s*\([^]]*\))?\s*\]\s*$/;
const ATTRIBUTE = /^\s*#\[[^\]]+\]\s*$/;
const FUNCTION =
  /^\s*(?:pub(?:\([^)]*\))?\s+)?(?:const\s+)?(?:async\s+)?(?:unsafe\s+)?fn\s+(?:r#)?([A-Za-z_][A-Za-z0-9_]*)\b/;
const SINGLE_WORD = /^[a-z][a-z0-9]*$/;

export function inspectLeaves(source: string, path: string): LeafFault[] {
  const faults: LeafFault[] = [];
  let attributed = false;

  for (const [index, line] of source.split(/\r?\n/).entries()) {
    if (TEST_ATTRIBUTE.test(line)) {
      attributed = true;
      continue;
    }
    if (!attributed) {
      continue;
    }
    if (line.trim() === "" || ATTRIBUTE.test(line)) {
      continue;
    }

    const match = line.match(FUNCTION);
    if (match && !SINGLE_WORD.test(match[1])) {
      faults.push({ path, line: index + 1, name: match[1] });
    }
    attributed = false;
  }

  return faults;
}

export async function scanLeaves(root: string): Promise<LeafFault[]> {
  const faults: LeafFault[] = [];
  const prefix = root.endsWith("/") ? root : `${root}/`;

  async function visit(directory: string): Promise<void> {
    for await (const entry of Deno.readDir(directory)) {
      const path = join(directory, entry.name);
      if (entry.isDirectory) {
        await visit(path);
      } else if (entry.isFile && entry.name.endsWith(".rs")) {
        const relative = path.startsWith(prefix) ? path.slice(prefix.length) : path;
        faults.push(...inspectLeaves(await Deno.readTextFile(path), relative));
      }
    }
  }

  await visit(join(root, "crates"));
  return faults.sort((left, right) =>
    left.path.localeCompare(right.path) || left.line - right.line ||
    left.name.localeCompare(right.name)
  );
}
