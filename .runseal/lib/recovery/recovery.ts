//! Operator-facing recovery capsule commands plus the two deploy integration
//! points. The host script owns validation and all state transitions.

import { runRecoveryRemote } from "@/lib/recovery/remote.ts";
import { command } from "@perish/sealkit/operator";

export type RecoveryRequest =
  | { action: "status" }
  | { action: "repair" }
  | { action: "execute"; capsule: string; candidateVersion: string }
  | { action: "accept"; capsule: string };

const COMMANDS = [
  { action: "status", tokens: ["status"] },
  { action: "repair", tokens: ["repair"] },
  { action: "accept", tokens: ["accept", { capture: "capsule" }] },
  {
    action: "execute",
    tokens: [
      "execute",
      { capture: "capsule" },
      "--confirm",
      { capture: "candidateVersion" },
    ],
  },
] as const;

export function parseRecoveryRequest(argv: string[]): RecoveryRequest {
  const request = command.parse(argv, COMMANDS);
  if (request.action === "status") {
    return { action: "status" };
  }
  if (request.action === "repair") {
    return { action: "repair" };
  }
  if (request.action === "accept") {
    return { action: "accept", capsule: request.values.capsule };
  }
  return {
    action: "execute",
    capsule: request.values.capsule,
    candidateVersion: request.values.candidateVersion,
  };
}

function printHelp(): void {
  console.log("Usage:");
  console.log("  runseal :rollback status");
  console.log("  runseal :rollback repair");
  console.log("  runseal :rollback execute <capsule-id> --confirm <candidate-version>");
  console.log("  runseal :rollback accept <capsule-id>");
  console.log("");
  console.log("Inspect, execute, or accept the single armed post-deploy recovery capsule.");
}

export async function recovery(argv: string[]): Promise<number> {
  if (argv.includes("-h") || argv.includes("--help")) {
    printHelp();
    return 0;
  }

  let request: RecoveryRequest;
  try {
    request = parseRecoveryRequest(argv);
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    printHelp();
    return 2;
  }

  if (request.action === "status") {
    return await runRecoveryRemote(["status"]);
  }
  if (request.action === "repair") {
    return await runRecoveryRemote(["arm"]);
  }
  if (request.action === "accept") {
    return await runRecoveryRemote(["accept", request.capsule]);
  }
  return await runRecoveryRemote([
    "execute",
    request.capsule,
    "--confirm",
    request.candidateVersion,
  ]);
}

/** Refuse a deployment while a previous candidate is still armed. */
export async function guardDeploy(): Promise<number> {
  return await runRecoveryRemote(["guard-deploy"]);
}

/** Turn the upgrader's raw pre-deploy snapshot into a durable capsule. */
export async function armRecovery(): Promise<number> {
  return await runRecoveryRemote(["arm"]);
}
