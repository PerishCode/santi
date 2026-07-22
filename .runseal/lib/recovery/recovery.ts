//! Operator-facing recovery capsule commands plus the two deploy integration
//! points. The host script owns validation and all state transitions.

import { runRecoveryRemote } from "@/lib/recovery/remote.ts";

const SAFE_ARGUMENT = /^[A-Za-z0-9._:+~-]+$/;

function requireSafeArgument(value: string): string {
  if (!SAFE_ARGUMENT.test(value)) {
    throw new Error(`invalid recovery argument: ${JSON.stringify(value)}`);
  }
  return value;
}

export type RecoveryRequest =
  | { action: "status" }
  | { action: "repair" }
  | { action: "execute"; capsule: string; candidateVersion: string }
  | { action: "accept"; capsule: string };

export function parseRecoveryRequest(argv: string[]): RecoveryRequest {
  if (argv.length === 1 && argv[0] === "status") {
    return { action: "status" };
  }
  if (argv.length === 1 && argv[0] === "repair") {
    return { action: "repair" };
  }
  if (argv.length === 2 && argv[0] === "accept") {
    return { action: "accept", capsule: requireSafeArgument(argv[1]) };
  }
  if (
    argv.length === 4 && argv[0] === "execute" && argv[2] === "--confirm" &&
    argv[1] !== "" && argv[3] !== ""
  ) {
    return {
      action: "execute",
      capsule: requireSafeArgument(argv[1]),
      candidateVersion: requireSafeArgument(argv[3]),
    };
  }
  throw new Error("invalid recovery command");
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
