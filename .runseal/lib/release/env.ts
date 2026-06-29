//! Shared release helpers: fail-fast, required env, public URL, GitHub outputs.

export function fail(message: string): never {
  console.error(`[release] ${message}`);
  Deno.exit(1);
}

export function required(name: string): string {
  const value = Deno.env.get(name);
  if (value === undefined || value.trim() === "") fail(`${name} is required`);
  return value;
}

/** Public releases base URL, normalized to an https origin with no trailing slash. */
export function publicUrl(): string {
  let url = required("SANTI_RELEASES_PUBLIC_URL").trim().replace(/\/+$/, "");
  if (!/^https?:\/\//.test(url)) url = `https://${url}`;
  return url;
}

export function writeOutput(name: string, value: string): void {
  const path = Deno.env.get("GITHUB_OUTPUT");
  if (path) Deno.writeTextFileSync(path, `${name}=${value}\n`, { append: true });
}

export function appendSummary(lines: string[]): void {
  const path = Deno.env.get("GITHUB_STEP_SUMMARY");
  if (path) Deno.writeTextFileSync(path, `${lines.join("\n")}\n`, { append: true });
}
