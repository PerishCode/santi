import { capture } from "@/lib/std/cmd.ts";
import { join } from "@/lib/std/fs.ts";

const FILE = ".runseal/negentropy.version";

export async function verify(root: string): Promise<void> {
  const expected = (await Deno.readTextFile(join(root, FILE))).trim();
  if (expected === "") throw new Error(`negentropy: missing pinned version in ${FILE}`);
  const result = await capture("negentropy", ["--version"], { cwd: root });
  const actual = result.stdout.trim();
  if (result.code !== 0 || actual !== `negentropy ${expected}`) {
    throw new Error(`negentropy: expected ${expected}, got ${actual || result.stderr.trim()}`);
  }
}
