//! `runseal :santi <santi-args...>`
//!
//! Runs the santi HTTP client against the deployed instance. base_url + auth come
//! from `.local/secrets/santi.toml` (gitignored; seed .runseal/templates/secrets/
//! santi.toml): edge auth (authentik client_credentials → JWT) when configured,
//! else a static api_key. The wrapper only locates the instance + credential and
//! hands off — santi is a transport-only HTTP client.

import { run } from "@/lib/std/cmd.ts";
import { repoRoot } from "@/lib/std/repo.ts";

const SANTI_CONFIG = ".local/secrets/santi.toml"; // relative to the repo root

function usage(): void {
  console.log("Usage: runseal :santi <santi-args...>");
  console.log("");
  console.log(
    "Runs the santi client against the deployed instance (config .local/secrets/santi.toml).",
  );
  console.log("Examples:");
  console.log("  runseal :santi health");
  console.log('  runseal :santi strand send <id> "hello"');
}

const args = [...Deno.args];
if (args.length === 0) {
  usage();
  Deno.exit(0);
}

const configPath = `${repoRoot()}/${SANTI_CONFIG}`;
let text: string;
try {
  text = await Deno.readTextFile(configPath);
} catch (err) {
  if (err instanceof Deno.errors.NotFound) {
    console.error(
      `:santi: missing client config ${SANTI_CONFIG} (seed .runseal/templates/secrets/santi.toml)`,
    );
    Deno.exit(1);
  }
  throw err;
}

const field = (name: string) => text.match(new RegExp(`^\\s*${name}\\s*=\\s*"([^"]+)"`, "m"))?.[1];

const baseUrl = field("base_url");
if (baseUrl === undefined) {
  console.error(`:santi: base_url missing in ${SANTI_CONFIG}`);
  Deno.exit(1);
}

// Prefer edge auth (authentik client_credentials) when fully configured; santi
// behind forward-auth needs it. Fall back to a static api_key otherwise.
const authArgs: string[] = [];
const tokenUrl = field("auth_token_url"), clientId = field("auth_client_id");
const username = field("auth_username"), password = field("auth_password");
if (tokenUrl && clientId && username && password) {
  authArgs.push(
    "--auth-token-url",
    tokenUrl,
    "--auth-client-id",
    clientId,
    "--auth-username",
    username,
    "--auth-password",
    password,
  );
} else {
  const apiKey = field("api_key");
  if (apiKey === undefined) {
    console.error(`:santi: config needs auth_* (client_credentials) or api_key in ${SANTI_CONFIG}`);
    Deno.exit(1);
  }
  authArgs.push("--api-key", apiKey);
}

const code = await run("santi", ["--base-url", baseUrl, ...authArgs, ...args]);
Deno.exit(code);
