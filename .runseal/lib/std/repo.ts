//! Repo root resolution, shared by wrappers.

/** The directory containing the active runseal profile (the repo root). */
export function repoRoot(): string {
  const profile = Deno.env.get("RUNSEAL_PROFILE_PATH");
  return profile ? dirname(profile) : Deno.cwd();
}

function dirname(path: string): string {
  const trimmed = path.replace(/\/+$/, "");
  const index = trimmed.lastIndexOf("/");
  return index <= 0 ? "/" : trimmed.slice(0, index);
}
