//! Minimal process primitives over Deno-native APIs. Discovery is `ps`-based:
//! the process table is the authority, never a pidfile.

export interface PsRow {
  pid: number;
  command: string;
}

/** Snapshot the process table as (pid, full command line) rows. Unix only. */
export async function psList(): Promise<PsRow[]> {
  const output = await new Deno.Command("ps", {
    args: ["-axo", "pid=,command="],
    stdout: "piped",
    stderr: "null",
  }).output();
  if (!output.success) {
    throw new Error("ps failed");
  }
  const text = new TextDecoder().decode(output.stdout);
  const rows: PsRow[] = [];
  for (const line of text.split("\n")) {
    const trimmed = line.trimStart();
    if (trimmed === "") continue;
    const space = trimmed.search(/\s/);
    if (space < 0) continue;
    const pid = Number(trimmed.slice(0, space));
    if (!Number.isInteger(pid)) continue;
    rows.push({ pid, command: trimmed.slice(space + 1).trim() });
  }
  return rows;
}

/** Send SIGTERM; returns false if the process is already gone. */
export function term(pid: number): boolean {
  try {
    Deno.kill(pid, "SIGTERM");
    return true;
  } catch {
    return false;
  }
}

/** Send SIGKILL; returns false if the process is already gone. */
export function killHard(pid: number): boolean {
  try {
    Deno.kill(pid, "SIGKILL");
    return true;
  } catch {
    return false;
  }
}
