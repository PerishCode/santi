//! Tiny filesystem path helpers, shared by wrappers.

export function exists(path: string): boolean {
  try {
    Deno.statSync(path);
    return true;
  } catch {
    return false;
  }
}

export function join(...parts: string[]): string {
  return parts.join("/").replace(/(?<!:)\/{2,}/g, "/");
}
