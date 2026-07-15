//! Registry-drift gate, address-bound (Liberte CI-image amendment v3,
//! 2026-07-15).
//!
//! NOT a pinned container, and NOT machine attestation: this Forge version
//! rejects digest-qualified job containers before dispatch AND exposes no
//! stable runner identity to the job (runner.name interpolates empty). The
//! CI pool members each build and serve their own image behind an
//! operator-managed /etc/hosts mapping of the registry hostname to the
//! hosting appliance's own address. That mapping — the same channel the
//! observation travels over — selects the authority member BEFORE the
//! registry is observed, and every request is FORCED to the selected
//! address (curl --resolve) with the hostname preserved for TLS
//! verification; the effective peer address, redirect count, content-digest
//! header, independently hashed manifest body, manifest-named config digest,
//! and platform are all verified fail-closed. One member drifting to the
//! other's otherwise-approved image still fails: selection is by address,
//! never by whichever digest matches. This does not attest the launched
//! container or the physical machine; stale-cache, race, compromised-image/
//! runner/registry limitations remain. The authority changes only through
//! reviewed source changes; CI never writes it. Sunset: restore a genuinely
//! immutable reference once the Forge accepts digest-qualified containers or
//! CI publishes never-reused versioned tags.

export interface Member {
  address: string;
  digest: string;
}

export interface Authority {
  schema: string;
  registry: string;
  repository: string;
  tag: string;
  media: string;
  platform: string;
  members: Record<string, Member>;
}

export const AUTHORITY_FILE = ".forgejo/ci-image.digest";
export const WORKFLOW_FILE = ".forgejo/workflows/guard.yml";
export const HOSTS_FILE = "/etc/hosts";

const FIELDS = ["schema", "registry", "repository", "tag", "media", "platform", "members"];
const IPV4 = /^(?:(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)\.){3}(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)$/;

export function loadAuthority(text: string): Authority {
  const authority = JSON.parse(text) as Authority;
  if (authority.schema !== "santi.ci_image.v3") {
    throw new Error(`${AUTHORITY_FILE}: not a santi.ci_image.v3 document`);
  }
  for (const field of ["registry", "repository", "tag", "media", "platform"] as const) {
    if (typeof authority[field] !== "string" || authority[field].length === 0) {
      throw new Error(`${AUTHORITY_FILE}: missing field ${field}`);
    }
  }
  for (const key of Object.keys(authority)) {
    if (!FIELDS.includes(key)) {
      throw new Error(`${AUTHORITY_FILE}: unknown field ${key}`);
    }
  }
  const members = authority.members;
  if (!members || typeof members !== "object" || Object.keys(members).length === 0) {
    throw new Error(`${AUTHORITY_FILE}: member map is missing or empty`);
  }
  const addresses = new Set<string>();
  for (const [label, entry] of Object.entries(members)) {
    if (!entry || typeof entry !== "object") {
      throw new Error(`${AUTHORITY_FILE}: member ${label} is not an object`);
    }
    if (!IPV4.test(entry.address ?? "")) {
      throw new Error(`${AUTHORITY_FILE}: malformed address for member ${label}`);
    }
    if (!/^sha256:[0-9a-f]{64}$/.test(entry.digest ?? "")) {
      throw new Error(`${AUTHORITY_FILE}: malformed digest for member ${label}`);
    }
    if (addresses.has(entry.address)) {
      throw new Error(`${AUTHORITY_FILE}: duplicate member address ${entry.address}`);
    }
    addresses.add(entry.address);
    const spellings = text.match(new RegExp(`"${label}"\\s*:`, "g")) ?? [];
    if (spellings.length > 1) {
      throw new Error(`${AUTHORITY_FILE}: duplicate member identity ${label}`);
    }
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

/** The workflow selector must name exactly the authority's registry/repo:tag. */
export function agree(authority: Authority, chosen: string): string[] {
  const normalized = chosen.includes("@")
    ? chosen
    : /:[\w.-]+$/.test(chosen.split("/").pop() ?? "")
    ? chosen
    : `${chosen}:latest`;
  const wanted = `${authority.registry}/${authority.repository}:${authority.tag}`;
  if (normalized !== wanted) {
    return [
      `workflow container selector "${chosen}" (normalized "${normalized}") != authority "${wanted}"`,
    ];
  }
  return [];
}

/**
 * Structural /etc/hosts parse: exact hostname-token match only, comments and
 * blank lines ignored, exactly ONE entry line naming the hostname, with a
 * well-formed address. Anything else is a pre-network failure.
 */
export function mapping(hosts: string, hostname: string): { address?: string; problem?: string } {
  const found: string[] = [];
  for (const raw of hosts.split("\n")) {
    const line = raw.split("#")[0].trim();
    if (line.length === 0) {
      continue;
    }
    const tokens = line.split(/\s+/);
    const [address, ...names] = tokens;
    if (names.some((name) => name === hostname)) {
      found.push(address);
    }
  }
  if (found.length === 0) {
    return { problem: `${HOSTS_FILE} has no entry for ${hostname}` };
  }
  if (found.length > 1) {
    return {
      problem:
        `${HOSTS_FILE} names ${hostname} on ${found.length} lines (ambiguous; never collapsed)`,
    };
  }
  if (!IPV4.test(found[0])) {
    return { problem: `${HOSTS_FILE} maps ${hostname} to a malformed address "${found[0]}"` };
  }
  return { address: found[0] };
}

/** Select exactly one authority member by the mapped address. */
export function member(
  authority: Authority,
  address: string,
): { label?: string; entry?: Member; problem?: string } {
  for (const [label, entry] of Object.entries(authority.members)) {
    if (entry.address === address) {
      return { label, entry };
    }
  }
  return { problem: `address ${address} is not a reviewed member of ${AUTHORITY_FILE}` };
}

export interface Answer {
  status: number;
  remote: string;
  redirects: number;
  digest?: string;
  body: Uint8Array;
}

/** Endpoint-bound client: every request is forced to `address`. */
export type Client = (url: string, accept: string, address: string) => Promise<Answer>;

interface Manifest {
  schemaVersion?: number;
  mediaType?: string;
  manifests?: unknown;
  config?: { digest?: string };
  layers?: unknown[];
}

export interface Selection {
  label: string;
  address: string;
}

/**
 * Fail-closed, endpoint-bound verification. The member is selected before
 * any request; both requests are forced to the member address and the
 * effective peer, redirect count, digests, manifest form, config-blob
 * digest, and platform are verified.
 */
export async function verify(
  authority: Authority,
  chosen: Selection,
  client: Client,
): Promise<{ problems: string[]; evidence: string[] }> {
  const problems: string[] = [];
  const evidence: string[] = [];
  const entry = authority.members[chosen.label];
  evidence.push(`selected member ${chosen.label} at ${chosen.address}`);
  evidence.push(`expected digest ${entry.digest}`);

  const base = `https://${authority.registry}/v2/${authority.repository}`;
  let answer: Answer;
  try {
    answer = await client(`${base}/manifests/${authority.tag}`, authority.media, chosen.address);
  } catch (error) {
    problems.push(`registry unreachable: ${error instanceof Error ? error.message : error}`);
    return { problems, evidence };
  }
  if (answer.remote !== chosen.address) {
    problems.push(`effective peer ${answer.remote} != selected address ${chosen.address}`);
    return { problems, evidence };
  }
  evidence.push(`effective peer ${answer.remote}`);
  if (answer.redirects > 0) {
    problems.push(`registry redirected ${answer.redirects} time(s); redirects are refused`);
    return { problems, evidence };
  }
  if (answer.status !== 200) {
    problems.push(`registry answered HTTP ${answer.status} for the manifest`);
    return { problems, evidence };
  }
  if (answer.digest === undefined) {
    problems.push("registry response carries no docker-content-digest header");
  } else {
    evidence.push(`observed header digest ${answer.digest}`);
    if (answer.digest !== entry.digest) {
      problems.push(`header digest ${answer.digest} != member digest ${entry.digest}`);
    }
  }
  const hashed = `sha256:${await sha256(answer.body)}`;
  evidence.push(`observed body digest ${hashed}`);
  if (hashed !== entry.digest) {
    problems.push(`independently hashed manifest ${hashed} != member digest ${entry.digest}`);
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
    config = await client(
      `${base}/blobs/${manifest.config.digest}`,
      "application/json",
      chosen.address,
    );
  } catch (error) {
    problems.push(`config blob unreachable: ${error instanceof Error ? error.message : error}`);
    return { problems, evidence };
  }
  if (config.remote !== chosen.address) {
    problems.push(`config blob peer ${config.remote} != selected address ${chosen.address}`);
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

/** Terminal-phase continuity: selection must be unchanged from initial. */
export function unchanged(initial: Selection, terminal: Selection): string[] {
  if (initial.label !== terminal.label || initial.address !== terminal.address) {
    return [
      `selection moved during the job: initial ${initial.label}@${initial.address} != terminal ${terminal.label}@${terminal.address}`,
    ];
  }
  return [];
}

async function sha256(bytes: Uint8Array): Promise<string> {
  const hash = await crypto.subtle.digest("SHA-256", bytes as BufferSource);
  return Array.from(new Uint8Array(hash))
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("");
}
