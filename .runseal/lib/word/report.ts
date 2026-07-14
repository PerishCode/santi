//! Parse `negentropy --strict --debt` output and bucket word findings by track.

import { capture } from "@/lib/std/cmd.ts";
import { join } from "@/lib/std/fs.ts";
import {
  assertFresh,
  declarationCount,
  Evidence,
  EVIDENCE_FILE,
  exportContract,
  fieldBinding,
  loadEvidence,
} from "@/lib/word/schema.ts";
import { classify, Track, TRACKS } from "@/lib/word/tracks.ts";

export interface Finding {
  path: string;
  line: number;
  column: number;
  token: string;
  track: Track;
}

export interface TrackStats {
  occurrences: number;
  unique: number;
  hapax: number;
  top: Array<{ token: string; count: number }>;
}

export interface Report {
  total: number;
  structural: number;
  tracks: Record<Track, TrackStats>;
  findings: Finding[];
}

export type BaselineTrack = "wire" | "test" | "internal";

export const BASELINE_TRACKS: readonly BaselineTrack[] = ["wire", "test", "internal"];

export interface Baseline {
  schema: string;
  note: string;
  total: number;
  tracks: Record<BaselineTrack, { occurrences: number; unique: number }>;
}

const DEBT_LINE = /^(.+?):(\d+):(\d+) debt (.+)$/;
/** Path-segment word debt has no line/column. */
const DEBT_PATH = /^(.+?) debt (.+)$/;
const SUMMARY = /^(\d+) faults, (\d+) blindspots, (\d+) debt$/;
const BY_LAW = /^by law: (.+)$/;
const BASELINE_FILE = ".runseal/word-debt-baseline.json";

export async function scan(root: string): Promise<{ report: Report; raw: string }> {
  const result = await capture("negentropy", ["--strict", "--debt", "."], { cwd: root });
  if (result.code !== 0) {
    throw new Error(
      `negentropy failed (${result.code}): ${result.stderr.trim() || result.stdout.trim()}`,
    );
  }
  const raw = result.stdout;
  return { report: parse(raw), raw };
}

export function parse(raw: string): Report {
  const findings: Finding[] = [];
  let structural = 0;
  for (const line of raw.split("\n")) {
    const located = line.match(DEBT_LINE);
    if (located) {
      const path = located[1];
      const lineNo = Number(located[2]);
      const column = Number(located[3]);
      const token = located[4];
      if (isStructural(token)) {
        structural += 1;
        continue;
      }
      findings.push({
        path,
        line: lineNo,
        column,
        token,
        track: classify(path, token),
      });
      continue;
    }
    const pathOnly = line.match(DEBT_PATH);
    if (pathOnly && !line.includes("faults,") && !line.startsWith("by law:")) {
      const path = pathOnly[1];
      const token = pathOnly[2];
      if (isStructural(token)) {
        structural += 1;
        continue;
      }
      // Path stems are naming pressure on module layout; seat as core.
      findings.push({
        path,
        line: 0,
        column: 0,
        token,
        track: path.includes("/tests/") ? "test" : "core",
      });
    }
  }

  return {
    total: findings.length,
    structural,
    tracks: trackStats(findings),
    findings,
  };
}

function trackStats(findings: Finding[]): Record<Track, TrackStats> {
  const tracks = emptyTracks();
  for (const track of TRACKS) {
    const bucket = findings.filter((finding) => finding.track === track);
    tracks[track] = stats(bucket);
  }
  return tracks;
}

/**
 * Scan, then verify the checked-in OpenAPI wire-type evidence against a fresh
 * contract export and apply identity bindings (W3 slice 1). Fails closed with
 * a regenerate instruction when the evidence is missing or stale.
 */
export async function classifiedScan(
  root: string,
): Promise<{ report: Report; raw: string; notes: string[] }> {
  const { report, raw } = await scan(root);
  const contract = await exportContract(root);
  let evidence: Evidence;
  try {
    evidence = await loadEvidence(root);
  } catch (error) {
    if (error instanceof Deno.errors.NotFound) {
      throw new Error(
        `missing ${EVIDENCE_FILE}; run: runseal :word-debt --sync-wire-schema`,
      );
    }
    throw error;
  }
  assertFresh(evidence, contract);
  const notes = await applyWireEvidence(root, report, evidence);
  return { report, raw, notes };
}

/**
 * Reclassify rows carrying contract identity bindings: PascalCase tokens that
 * ARE components (unique declaration), and snake field declarations that ARE
 * component properties (live owner-span verification, W3 slice 2). Ambiguity
 * declines loudly; locals, methods, and renamed fields never lift.
 */
export async function applyWireEvidence(
  root: string,
  report: Report,
  evidence: Evidence,
  countDeclarations: (root: string, token: string) => Promise<number> = declarationCount,
  readSource: (path: string) => Promise<string> = (path) => Deno.readTextFile(join(root, path)),
): Promise<string[]> {
  const set = new Set(evidence.components);
  const notes: string[] = [];
  const candidates = [
    ...new Set(
      report.findings
        .filter((finding) =>
          finding.track === "core" && /^[A-Z]/.test(finding.token) && set.has(finding.token)
        )
        .map((finding) => finding.token),
    ),
  ];
  let changed = false;
  for (const token of candidates) {
    const count = await countDeclarations(root, token);
    if (count === 1) {
      for (const finding of report.findings) {
        if (finding.token === token && finding.track === "core") {
          finding.track = "wire";
        }
      }
      changed = true;
      notes.push(
        `wire evidence: ${token} is an OpenAPI component with a unique declaration; core -> wire`,
      );
    } else {
      notes.push(
        `wire evidence declined for ${token}: ${count} workspace declarations (identity binding needs exactly 1); stays core`,
      );
    }
  }

  const propertyTokens = new Set(Object.values(evidence.properties).flat());
  const groups = new Map<string, Finding[]>();
  for (const finding of report.findings) {
    if (finding.track === "internal" && finding.line > 0 && propertyTokens.has(finding.token)) {
      const key = `${finding.path}\u0000${finding.token}`;
      groups.set(key, [...(groups.get(key) ?? []), finding]);
    }
  }
  const sources = new Map<string, string>();
  for (const rows of groups.values()) {
    let source = sources.get(rows[0].path);
    if (source === undefined) {
      source = await readSource(rows[0].path);
      sources.set(rows[0].path, source);
    }
    const verdicts = rows.map((finding) => ({
      finding,
      verdict: fieldBinding(
        source as string,
        evidence.components,
        evidence.properties,
        finding.token,
        finding.line,
      ),
    }));
    const anyOwned = verdicts.some((entry) => entry.verdict.component !== undefined);
    const failures = verdicts.filter((entry) => !entry.verdict.lift);
    if (failures.length > 0) {
      if (anyOwned) {
        for (const entry of failures) {
          notes.push(
            `field evidence declined at ${entry.finding.path}:${entry.finding.line}: ${entry.verdict.reason}; group stays internal`,
          );
        }
        for (const entry of verdicts.filter((each) => each.verdict.lift)) {
          notes.push(
            `field evidence declined at ${entry.finding.path}:${entry.finding.line}: another ${entry.finding.token} row in this file failed verification; group stays internal`,
          );
        }
      }
      continue;
    }
    const components = [...new Set(verdicts.map((entry) => entry.verdict.component ?? ""))];
    if (components.length !== verdicts.length) {
      for (const entry of verdicts) {
        notes.push(
          `field evidence declined at ${entry.finding.path}:${entry.finding.line}: two rows resolve to the same owner (not injective); group stays internal`,
        );
      }
      continue;
    }
    let owned = true;
    for (const component of components) {
      const owners = await countDeclarations(root, component);
      if (owners !== 1) {
        owned = false;
        for (const entry of verdicts) {
          notes.push(
            `field evidence declined at ${entry.finding.path}:${entry.finding.line}: ${owners} workspace declarations of ${component}; group stays internal`,
          );
        }
        break;
      }
    }
    if (!owned) {
      continue;
    }
    for (const entry of verdicts) {
      entry.finding.track = "wire";
      notes.push(
        `field evidence: ${entry.verdict.reason}; internal -> wire (${entry.finding.path}:${entry.finding.line})`,
      );
    }
    changed = true;
  }

  if (changed) {
    report.tracks = trackStats(report.findings);
  }
  return notes;
}

function isStructural(token: string): boolean {
  return token.includes("parameters over") || /:\s*&/.test(token) || token.includes(":&");
}

export async function loadBaseline(root: string): Promise<Baseline> {
  const text = await Deno.readTextFile(join(root, BASELINE_FILE));
  return JSON.parse(text) as Baseline;
}

export function baselinePath(): string {
  return BASELINE_FILE;
}

export function formatReport(report: Report): string {
  const lines: string[] = [];
  lines.push(
    `word debt: ${report.total} occurrences (structural debt rows skipped: ${report.structural})`,
  );
  for (const track of TRACKS) {
    const stats = report.tracks[track];
    lines.push(
      `  ${track.padEnd(8)} occ=${stats.occurrences} unique=${stats.unique} hapax=${stats.hapax}`,
    );
    if (stats.top.length > 0) {
      const head = stats.top
        .slice(0, 8)
        .map((entry) => `${entry.token}×${entry.count}`)
        .join(", ");
      lines.push(`           top: ${head}`);
    }
  }
  return lines.join("\n");
}

/**
 * The word-debt gate (C4, Liberte 2026-07-14): core hard-fails on ANY
 * occurrence, independent of any baseline; internal rising above the
 * baseline warns loudly but never blocks; total stays observational.
 * The returned code IS the command exit outcome.
 */
export function ratchet(
  report: Report,
  baseline: Baseline,
): { code: number; messages: string[] } {
  const messages: string[] = [];
  let code = 0;

  if (report.total > baseline.total) {
    messages.push(
      `total word debt rose: ${report.total} > baseline ${baseline.total} (observed; not a hard gate)`,
    );
  } else if (report.total < baseline.total) {
    messages.push(
      `total word debt fell: ${report.total} < baseline ${baseline.total} (consider refreshing baseline)`,
    );
  }

  const core = report.findings.filter((finding) => finding.track === "core");
  if (core.length > 0) {
    code = 1;
    messages.push(
      `core gate FAILED: ${core.length} occurrence(s) of unexplained core debt — the gate is zero`,
    );
    for (const finding of core.slice(0, 20)) {
      messages.push(`  ${finding.token} at ${finding.path}:${finding.line}`);
    }
    if (core.length > 20) {
      messages.push(
        `  … ${core.length - 20} more; run: runseal :word-debt --core-top for the full inventory`,
      );
    }
  }

  const internal = report.tracks.internal.occurrences;
  const internalBase = baseline.tracks.internal.occurrences;
  if (internal > internalBase) {
    messages.push(
      `internal track ROSE: ${internal} > baseline ${internalBase} (+${
        internal - internalBase
      }) per ${BASELINE_FILE} — non-blocking; burn it back or seek approval to rebase`,
    );
  } else if (internal < internalBase) {
    messages.push(
      `internal track fell: ${internal} < baseline ${internalBase} (lower the baseline in the same batch)`,
    );
  }

  if (code === 0) {
    messages.push("word-debt gate: ok (core = 0)");
  }
  return { code, messages };
}

export function toBaseline(report: Report, note: string): Baseline {
  const tracks = {} as Baseline["tracks"];
  for (const track of BASELINE_TRACKS) {
    tracks[track] = {
      occurrences: report.tracks[track].occurrences,
      unique: report.tracks[track].unique,
    };
  }
  return {
    schema: "santi.word_debt.v1",
    note,
    total: report.total,
    tracks,
  };
}

export function coreTop(report: Report, limit = 50): Array<{ token: string; count: number }> {
  return report.tracks.core.top.slice(0, limit);
}

function emptyTracks(): Record<Track, TrackStats> {
  const out = {} as Record<Track, TrackStats>;
  for (const track of TRACKS) {
    out[track] = { occurrences: 0, unique: 0, hapax: 0, top: [] };
  }
  return out;
}

function stats(findings: Finding[]): TrackStats {
  const counts = new Map<string, number>();
  for (const finding of findings) {
    counts.set(finding.token, (counts.get(finding.token) ?? 0) + 1);
  }
  let hapax = 0;
  const top: Array<{ token: string; count: number }> = [];
  for (const [token, count] of counts) {
    if (count === 1) hapax += 1;
    top.push({ token, count });
  }
  top.sort((left, right) => right.count - left.count || left.token.localeCompare(right.token));
  return {
    occurrences: findings.length,
    unique: counts.size,
    hapax,
    top,
  };
}

export function sampleHapax(
  report: Report,
  track: Track,
  n: number,
  seed = 1,
): Finding[] {
  const hapaxTokens = new Set(
    report.tracks[track].top.filter((entry) => entry.count === 1).map((entry) => entry.token),
  );
  const pool = report.findings.filter(
    (finding) => finding.track === track && hapaxTokens.has(finding.token),
  );
  // Deterministic shuffle (LCG).
  let state = seed >>> 0;
  const copy = pool.slice();
  for (let i = copy.length - 1; i > 0; i -= 1) {
    state = (1664525 * state + 1013904223) >>> 0;
    const j = state % (i + 1);
    [copy[i], copy[j]] = [copy[j], copy[i]];
  }
  return copy.slice(0, n);
}

/** Expose law summary lines for operators. */
export function lawLines(raw: string): string[] {
  return raw.split("\n").filter((line) => SUMMARY.test(line) || BY_LAW.test(line));
}
