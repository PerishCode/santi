//! Minimal repo-local SSH config reader. Operates only on the repo-local
//! `.local/ssh/config`; arbitrary -F/-i overrides are intentionally unsupported.
//! Ported into santi so its own remote-ops wrappers carry no dependency on the
//! infra repo (santi owns its high-frequency ops; infra owns the cold buildout).

async function lines(configPath: string): Promise<string[]> {
  const text = await Deno.readTextFile(configPath);
  return text.split(/\r?\n/);
}

/** Host aliases explicitly declared by a `Host` line, excluding wildcards. */
export async function declaredHosts(configPath: string): Promise<string[]> {
  const hosts: string[] = [];
  for (const line of await lines(configPath)) {
    const match = line.match(/^\s*Host\s+(.+?)\s*$/i);
    if (match === null) {
      continue;
    }
    for (const token of match[1].split(/\s+/)) {
      if (token === "" || token.includes("*") || token.includes("?")) {
        continue;
      }
      hosts.push(token);
    }
  }
  return hosts;
}

export async function hostDeclared(configPath: string, host: string): Promise<boolean> {
  return (await declaredHosts(configPath)).includes(host);
}
