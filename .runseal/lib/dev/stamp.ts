//! The arg-stamp: a packed identity injected into the launcher process's argv
//! so process management can discover and operate on it via `ps`.
//!
//! The stamp is the single source of truth for "is santi running, and which
//! one". It rides on the process command line (not a file), so it cannot drift
//! out of sync with reality the way a pidfile can. Any cache file is secondary.
//!
//! Canonical form: `--santi-stamp=v=1;a=santi-api;n=<namespace>;port=<port>`.
//! Values are percent-encoded so the whole token stays whitespace-free and
//! survives `ps` command-line splitting.

export const STAMP_FLAG = "--santi-stamp";
export const STAMP_VERSION = 1;
export const APP = "santi";
export const DEFAULT_NAMESPACE = "default";

export interface Stamp {
  version: number;
  app: string;
  namespace: string;
  port: number;
}

export function encodeStamp(stamp: Stamp): string {
  return [
    `v=${stamp.version}`,
    `a=${encodeValue(stamp.app)}`,
    `n=${encodeValue(stamp.namespace)}`,
    `port=${stamp.port}`,
  ].join(";");
}

export function stampArg(stamp: Stamp): string {
  return `${STAMP_FLAG}=${encodeStamp(stamp)}`;
}

export function decodeStamp(value: string): Stamp | null {
  const fields: Record<string, string> = {};
  for (const part of value.split(";")) {
    const index = part.indexOf("=");
    if (index < 0) return null;
    fields[part.slice(0, index)] = decodeValue(part.slice(index + 1));
  }
  if (fields.v === undefined || fields.a === undefined || fields.n === undefined) return null;
  const version = Number(fields.v);
  if (!Number.isInteger(version) || version !== STAMP_VERSION) return null;
  const port = Number(fields.port ?? "0");
  return {
    version,
    app: fields.a,
    namespace: fields.n,
    port: Number.isFinite(port) ? port : 0,
  };
}

/** Extract a stamp from a full `ps` command line, if one is present. */
export function readStampFromCommand(command: string): Stamp | null {
  const match = command.match(/--santi-stamp=(\S+)/);
  return match ? decodeStamp(match[1]) : null;
}

function encodeValue(value: string): string {
  return value.replace(
    /[^A-Za-z0-9._-]/g,
    (char) => `%${char.charCodeAt(0).toString(16).toUpperCase().padStart(2, "0")}`,
  );
}

function decodeValue(value: string): string {
  return value.replace(/%([0-9A-Fa-f]{2})/g, (_, hex) => String.fromCharCode(parseInt(hex, 16)));
}
