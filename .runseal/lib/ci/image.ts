//! Registry-integrity gate on a never-reused versioned tag (Liberte sunset
//! ruling v4, 2026-07-16).
//!
//! The workflow selector pins an operator-enforced, NEVER-REUSED versioned
//! tag, published and digest-converged across every pool registry (infra
//! !129/!137: pushv + seed-tags.py + mandatory CONVERGE runbook step). This
//! gate makes the discipline mechanically verifiable: initial and terminal
//! phases each check the registry's manifest for exactly that tag against
//! the reviewed authority digest (content-digest header AND independently
//! hashed raw body), the manifest form, the config blob (hashed against the
//! manifest-named digest before its platform fields are trusted), and
//! linux/amd64. The terminal phase additionally requires the authority,
//! selector, and RUNNER_NAME unchanged since the initial record.
//!
//! Accurate characterization (ruled): an operator-enforced versioned-tag
//! reference supplemented by registry-integrity checks — NOT
//! content-addressed execution, NOT container or runner attestation, NOT
//! proof against a compromised image, runner, or registry. Stale-cache and
//! resolution-race limitations remain. If the Forge gains digest-qualified
//! job containers, propose the literal digest reference. RUNNER_NAME is
//! audit contract only (infra-injected, must be present and stable); it
//! never selects authority. CI never writes the authority file.

export interface Authority {
  schema: string;
  registry: string;
  repository: string;
  tag: string;
  digest: string;
  media: string;
  platform: string;
}

export const AUTHORITY_FILE = ".forgejo/ci-image.digest";
export const WORKFLOW_FILE = ".forgejo/workflows/guard.yml";

const FIELDS = ["schema", "registry", "repository", "tag", "digest", "media", "platform"];
const TAG = /^\d{8}-[0-9a-f]{8}$/;

export function loadAuthority(text: string): Authority {
  const authority = JSON.parse(text) as Authority;
  if (authority.schema !== "santi.ci_image.v4") {
    throw new Error(`${AUTHORITY_FILE}: not a santi.ci_image.v4 document`);
  }
  for (const field of FIELDS) {
    const value = authority[field as keyof Authority];
    if (typeof value !== "string" || value.length === 0) {
      throw new Error(`${AUTHORITY_FILE}: missing field ${field}`);
    }
  }
  for (const key of Object.keys(authority)) {
    if (!FIELDS.includes(key)) {
      throw new Error(`${AUTHORITY_FILE}: unknown field ${key}`);
    }
  }
  if (!/^sha256:[0-9a-f]{64}$/.test(authority.digest)) {
    throw new Error(`${AUTHORITY_FILE}: malformed digest`);
  }
  if (!TAG.test(authority.tag)) {
    throw new Error(
      `${AUTHORITY_FILE}: tag "${authority.tag}" is outside the reviewed <yyyymmdd>-<digest8> format (mutable tags like latest are refused)`,
    );
  }
  const suffix = authority.tag.slice(9);
  if (suffix !== authority.digest.slice("sha256:".length, "sha256:".length + 8)) {
    throw new Error(
      `${AUTHORITY_FILE}: tag suffix ${suffix} does not locate the authority digest (the full digest is the authority; the suffix is a human locator)`,
    );
  }
  if (!/^[a-z0-9.-]+$/.test(authority.registry) || !/^[a-z0-9/._-]+$/.test(authority.repository)) {
    throw new Error(`${AUTHORITY_FILE}: malformed registry or repository`);
  }
  return authority;
}

/** Extract the job container selector from the workflow source. */
export function selector(workflow: string): string {
  const shorthand = workflow.match(/^[ \t]*container:[ \t]+([^\s{][^\n]*)$/m);
  if (shorthand) {
    return shorthand[1].trim();
  }
  const mapped = workflow.match(/^\s*container:\s*\n\s*image:\s*(\S+)/m);
  if (mapped) {
    return mapped[1].trim();
  }
  throw new Error(`${WORKFLOW_FILE}: no container selector found`);
}

/** The workflow selector must equal the authority reference EXACTLY. */
export function agree(authority: Authority, chosen: string): string[] {
  const wanted = `${authority.registry}/${authority.repository}:${authority.tag}`;
  if (chosen !== wanted) {
    return [
      `workflow container selector "${chosen}" != authority "${wanted}" (exact match required; no default or fallback tag)`,
    ];
  }
  return [];
}

export interface Answer {
  status: number;
  remote: string;
  redirects: number;
  digest?: string;
  body: Uint8Array;
}

/** Client contract: direct/no-proxy, redirects refused, peer reported. */
export type Client = (url: string, accept: string) => Promise<Answer>;

interface Manifest {
  schemaVersion?: number;
  mediaType?: string;
  manifests?: unknown;
  config?: { digest?: string };
  layers?: unknown[];
}

/**
 * Fail-closed registry-integrity verification of the versioned tag. The
 * effective peer address is diagnostic evidence only — it never selects
 * authority and never decides pass/fail.
 */
export async function verify(
  authority: Authority,
  runner: string | undefined,
  client: Client,
): Promise<{ problems: string[]; evidence: string[] }> {
  const problems: string[] = [];
  const evidence: string[] = [];
  if (runner === undefined || runner.length === 0 || runner.length > 64) {
    return {
      problems: [
        "RUNNER_NAME is missing or unusable; the infrastructure declares it part of the CI audit contract",
      ],
      evidence,
    };
  }
  evidence.push(`runner ${runner.replace(/[^\w.-]/g, "_")}`);
  evidence.push(`reference ${authority.registry}/${authority.repository}:${authority.tag}`);
  evidence.push(`expected digest ${authority.digest}`);

  const base = `https://${authority.registry}/v2/${authority.repository}`;
  let answer: Answer;
  try {
    answer = await client(`${base}/manifests/${authority.tag}`, authority.media);
  } catch (error) {
    problems.push(`registry unreachable: ${error instanceof Error ? error.message : error}`);
    return { problems, evidence };
  }
  evidence.push(`effective peer ${answer.remote} (diagnostic only)`);
  if (answer.redirects > 0) {
    problems.push(`registry redirected ${answer.redirects} time(s); redirects are refused`);
    return { problems, evidence };
  }
  if (answer.status !== 200) {
    problems.push(`registry answered HTTP ${answer.status} for the versioned tag`);
    return { problems, evidence };
  }
  if (answer.digest === undefined) {
    problems.push("registry response carries no docker-content-digest header");
  } else {
    evidence.push(`observed header digest ${answer.digest}`);
    if (answer.digest !== authority.digest) {
      problems.push(
        `header digest ${answer.digest} != authority ${authority.digest} (the versioned tag no longer serves its reviewed content)`,
      );
    }
  }
  const hashed = `sha256:${await sha256(answer.body)}`;
  evidence.push(`observed body digest ${hashed}`);
  if (hashed !== authority.digest) {
    problems.push(`independently hashed manifest ${hashed} != authority ${authority.digest}`);
  }
  let manifest: Manifest;
  try {
    manifest = JSON.parse(new TextDecoder().decode(answer.body)) as Manifest;
  } catch {
    problems.push("manifest body is not valid JSON");
    return { problems, evidence };
  }
  if (manifest.mediaType !== authority.media) {
    problems.push(`manifest mediaType ${manifest.mediaType} != pinned ${authority.media}`);
  }
  evidence.push(`observed mediaType ${manifest.mediaType}`);
  if (manifest.manifests !== undefined) {
    problems.push("registry returned an image INDEX; the gate pins a single-platform manifest");
    return { problems, evidence };
  }
  if (manifest.schemaVersion !== 2 || !manifest.config?.digest || !Array.isArray(manifest.layers)) {
    problems.push("manifest is not the expected schemaVersion-2 image-manifest form");
    return { problems, evidence };
  }
  let config: Answer;
  try {
    config = await client(`${base}/blobs/${manifest.config.digest}`, "application/json");
  } catch (error) {
    problems.push(`config blob unreachable: ${error instanceof Error ? error.message : error}`);
    return { problems, evidence };
  }
  if (config.redirects > 0 || config.status !== 200) {
    problems.push(
      `config blob answered HTTP ${config.status} with ${config.redirects} redirect(s)`,
    );
    return { problems, evidence };
  }
  const configHash = `sha256:${await sha256(config.body)}`;
  if (configHash !== manifest.config.digest) {
    problems.push(
      `config blob hash ${configHash} != manifest-named ${manifest.config.digest}; platform fields untrusted`,
    );
    return { problems, evidence };
  }
  try {
    const parsed = JSON.parse(new TextDecoder().decode(config.body)) as {
      architecture?: string;
      os?: string;
    };
    const platform = `${parsed.os}/${parsed.architecture}`;
    evidence.push(`observed platform ${platform}`);
    if (platform !== authority.platform) {
      problems.push(`platform ${platform} != pinned ${authority.platform}`);
    }
  } catch {
    problems.push("config blob is not valid JSON");
  }
  return { problems, evidence };
}

export interface Trace {
  authority: string;
  reference: string;
  digest: string;
  chosen: string;
  media: string;
  platform: string;
  runner: string;
}

/** The initial-phase evidence record (occurrence evidence, never authority). */
export async function snapshot(
  text: string,
  authority: Authority,
  chosen: string,
  runner: string,
): Promise<Trace> {
  return {
    authority: `sha256:${await sha256(new TextEncoder().encode(text))}`,
    reference: `${authority.registry}/${authority.repository}:${authority.tag}`,
    digest: authority.digest,
    chosen,
    media: authority.media,
    platform: authority.platform,
    runner,
  };
}

/** Terminal continuity: nothing may have moved since the initial record. */
export function unchanged(initial: Trace, terminal: Trace): string[] {
  const moved: string[] = [];
  for (const key of Object.keys(initial) as Array<keyof Trace>) {
    if (initial[key] !== terminal[key]) {
      moved.push(
        `${key} moved during the job: initial ${initial[key]} != terminal ${terminal[key]}`,
      );
    }
  }
  return moved;
}

async function sha256(bytes: Uint8Array): Promise<string> {
  const hash = await crypto.subtle.digest("SHA-256", bytes as BufferSource);
  return Array.from(new Uint8Array(hash))
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("");
}
