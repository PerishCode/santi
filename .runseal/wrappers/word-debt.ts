//! `runseal :word-debt` — report Liberte-track word debt and enforce the ratchet.
//!
//! Usage:
//!   runseal :word-debt              # report + ratchet (exit 1 if over baseline)
//!   runseal :word-debt --write-baseline
//!   runseal :word-debt --sample-hapax
//!   runseal :word-debt --core-top
//!   runseal :word-debt --sync-wire-schema

import { join } from "@/lib/std/fs.ts";
import { repoRoot } from "@/lib/std/repo.ts";
import {
  baselinePath,
  classifiedScan,
  coreTop,
  formatReport,
  lawLines,
  loadBaseline,
  ratchet,
  sampleHapax,
  toBaseline,
} from "@/lib/word/report.ts";
import { EVIDENCE_FILE, exportContract, writeEvidence } from "@/lib/word/schema.ts";

export async function main(argv: string[]): Promise<number> {
  if (argv.includes("-h") || argv.includes("--help")) {
    usage();
    return 0;
  }
  const root = repoRoot();

  if (argv.includes("--sync-wire-schema")) {
    const contract = await exportContract(root);
    await writeEvidence(root, contract);
    console.log(
      `wrote ${EVIDENCE_FILE}: ${contract.components.length} components, contract digest ${contract.digest}`,
    );
    return 0;
  }

  let report, raw, notes;
  try {
    ({ report, raw, notes } = await classifiedScan(root));
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    return 2;
  }
  for (const note of notes) {
    console.log(note);
  }

  for (const line of lawLines(raw)) {
    console.log(line);
  }
  console.log(formatReport(report));

  if (argv.includes("--core-top")) {
    console.log("core top:");
    for (const entry of coreTop(report, 50)) {
      console.log(`  ${String(entry.count).padStart(4)}  ${entry.token}`);
    }
  }

  if (argv.includes("--sample-hapax")) {
    console.log("hapax sample (internal, n=40, seed=7):");
    for (const finding of sampleHapax(report, "internal", 40, 7)) {
      console.log(`  ${finding.path}:${finding.line}  ${finding.token}`);
    }
    console.log("hapax sample (core, n=20, seed=7):");
    for (const finding of sampleHapax(report, "core", 20, 7)) {
      console.log(`  ${finding.path}:${finding.line}  ${finding.token}`);
    }
  }

  if (argv.includes("--write-baseline")) {
    const baseline = toBaseline(
      report,
      "C4 2026-07-14 (strand ss_0d11cd5a): core gate is zero, no core entry; internal no-rise; wire/test observed",
    );
    const path = join(root, baselinePath());
    await Deno.writeTextFile(path, `${JSON.stringify(baseline, null, 2)}\n`);
    console.log(`wrote ${baselinePath()}`);
    return 0;
  }

  try {
    const baseline = await loadBaseline(root);
    const result = ratchet(report, baseline);
    for (const message of result.messages) {
      console.log(message);
    }
    return result.code;
  } catch (error) {
    if (error instanceof Deno.errors.NotFound) {
      console.error(
        `word-debt: missing ${baselinePath()}; run: runseal :word-debt --write-baseline`,
      );
      return 2;
    }
    throw error;
  }
}

function usage(): void {
  console.log(
    "Usage: runseal :word-debt [--write-baseline] [--core-top] [--sample-hapax] [--sync-wire-schema]",
  );
  console.log("");
  console.log("Classify Negentropy word debt into Liberte tracks and ratchet total+core.");
}

if (import.meta.main) {
  Deno.exit(await main(Deno.args));
}
