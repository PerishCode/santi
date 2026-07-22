import { parseRecoveryRequest } from "@/lib/recovery/recovery.ts";
import { recoveryRemoteCommand } from "@/lib/recovery/remote.ts";

function assertEquals(actual: unknown, expected: unknown): void {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error(`actual ${JSON.stringify(actual)}, expected ${JSON.stringify(expected)}`);
  }
}

function assertThrows(callback: () => unknown, needle: string): void {
  try {
    callback();
  } catch (error) {
    const text = error instanceof Error ? error.message : String(error);
    if (!text.includes(needle)) {
      throw new Error(`error lacks ${JSON.stringify(needle)}: ${text}`);
    }
    return;
  }
  throw new Error(`expected an error containing ${JSON.stringify(needle)}`);
}

Deno.test("recovery CLI requires an explicit candidate confirmation", () => {
  assertEquals(parseRecoveryRequest(["status"]), { action: "status" });
  assertEquals(parseRecoveryRequest(["repair"]), { action: "repair" });
  assertEquals(parseRecoveryRequest(["accept", "a--b--20260722T120000Z"]), {
    action: "accept",
    capsule: "a--b--20260722T120000Z",
  });
  assertEquals(
    parseRecoveryRequest([
      "execute",
      "a--b--20260722T120000Z",
      "--confirm",
      "0.1.0-beta.54",
    ]),
    {
      action: "execute",
      capsule: "a--b--20260722T120000Z",
      candidateVersion: "0.1.0-beta.54",
    },
  );
  assertThrows(() => parseRecoveryRequest(["execute", "capsule"]), "invalid");
  assertThrows(() => parseRecoveryRequest(["accept", "x;id"]), "invalid recovery argument");
});

Deno.test("remote recovery command rejects shell syntax", () => {
  assertEquals(
    recoveryRemoteCommand(["execute", "old--new--time", "--confirm", "0.1.0-beta.54"]),
    "bash -s -- execute old--new--time --confirm 0.1.0-beta.54",
  );
  assertThrows(() => recoveryRemoteCommand(["x; id"]), "unsafe recovery argument");
  assertThrows(() => recoveryRemoteCommand(["$(id)"]), "unsafe recovery argument");
  assertEquals(recoveryRemoteCommand(["1:0.1.0~rc1-1"]), "bash -s -- 1:0.1.0~rc1-1");
});
