//! Versioned-tag registry-integrity gate vectors (Liberte sunset ruling v4).

import {
  agree,
  Answer,
  loadAuthority,
  selector,
  snapshot,
  unchanged,
  verify,
} from "@/lib/ci/image.ts";

function assert(condition: boolean, message = "assertion failed"): void {
  if (!condition) {
    throw new Error(message);
  }
}

function assertEquals(actual: unknown, expected: unknown, message = "values differ"): void {
  const left = JSON.stringify(actual);
  const right = JSON.stringify(expected);
  if (left !== right) {
    throw new Error(`${message}: actual ${left}, expected ${right}`);
  }
}

function assertThrows(callback: () => unknown, needle: string): void {
  try {
    callback();
  } catch (error) {
    const text = error instanceof Error ? error.message : String(error);
    assert(text.includes(needle), `error lacks "${needle}": ${text}`);
    return;
  }
  throw new Error(`expected an error containing "${needle}", none was thrown`);
}

const encoder = new TextEncoder();

async function digestOf(text: string): Promise<string> {
  const hash = await crypto.subtle.digest("SHA-256", encoder.encode(text));
  return `sha256:${
    Array.from(new Uint8Array(hash)).map((byte) => byte.toString(16).padStart(2, "0")).join("")
  }`;
}

const config = JSON.stringify({ architecture: "amd64", os: "linux" });
const configDigest = await digestOf(config);

const manifest = JSON.stringify({
  schemaVersion: 2,
  mediaType: "application/vnd.docker.distribution.manifest.v2+json",
  config: { digest: configDigest },
  layers: [{}],
});
const pinned = await digestOf(manifest);
const short = pinned.slice("sha256:".length, "sha256:".length + 8);

function authorityText(overrides: Partial<Record<string, string>> = {}): string {
  return JSON.stringify({
    schema: "santi.ci_image.v4",
    registry: "mirror.perish.lan",
    repository: "ci/deno",
    tag: `20260716-${short}`,
    digest: pinned,
    media: "application/vnd.docker.distribution.manifest.v2+json",
    platform: "linux/amd64",
    ...overrides,
  });
}

const authority = loadAuthority(authorityText());
const RUNNER = "hk-01-heavy";

function serving(overrides: Partial<Answer> = {}, blob: string = config) {
  return (url: string, _accept: string): Promise<Answer> => {
    const base: Answer = url.includes("/manifests/")
      ? {
        status: 200,
        remote: "43.251.225.113",
        redirects: 0,
        digest: pinned,
        body: encoder.encode(manifest),
      }
      : { status: 200, remote: "43.251.225.113", redirects: 0, body: encoder.encode(blob) };
    return Promise.resolve(url.includes("/manifests/") ? { ...base, ...overrides } : base);
  };
}

Deno.test("the canonical authority passes", async () => {
  const { problems, evidence } = await verify(authority, RUNNER, serving());
  assertEquals(problems, []);
  assert(evidence.some((line) => line.includes("runner hk-01-heavy")));
  assert(evidence.some((line) => line.includes("diagnostic only")));
  assert(evidence.some((line) => line.includes("observed platform linux/amd64")));
});

Deno.test("latest is rejected", () => {
  assertThrows(() => loadAuthority(authorityText({ tag: "latest" })), "outside the reviewed");
});

Deno.test("malformed version tags are rejected", () => {
  assertThrows(() => loadAuthority(authorityText({ tag: "v1.2.3" })), "outside the reviewed");
  assertThrows(
    () => loadAuthority(authorityText({ tag: `2026716-${short}` })),
    "outside the reviewed",
  );
});

Deno.test("tag suffix and full digest disagreement is rejected", () => {
  assertThrows(
    () => loadAuthority(authorityText({ tag: "20260716-00000000" })),
    "does not locate the authority digest",
  );
});

Deno.test("selector tag disagreement fails", () => {
  assert(agree(authority, "mirror.perish.lan/ci/deno:latest").length === 1);
  assert(agree(authority, "mirror.perish.lan/ci/deno").length === 1);
  assert(agree(authority, `mirror.perish.lan/ci/deno:20260716-00000000`).length === 1);
  assertEquals(agree(authority, `mirror.perish.lan/ci/deno:20260716-${short}`), []);
});

Deno.test("selector repository disagreement fails", () => {
  assert(agree(authority, `mirror.perish.lan/ci/rust:20260716-${short}`).length === 1);
  assert(agree(authority, `docker.io/ci/deno:20260716-${short}`).length === 1);
});

Deno.test("a changed header digest fails", async () => {
  const other = await digestOf("other");
  const { problems } = await verify(authority, RUNNER, serving({ digest: other }));
  assert(problems.some((line) => line.includes("no longer serves its reviewed content")));
});

Deno.test("a changed raw-body digest fails", async () => {
  const { problems } = await verify(
    authority,
    RUNNER,
    serving({ body: encoder.encode(`${manifest} `) }),
  );
  assert(problems.some((line) => line.includes("independently hashed manifest")));
});

Deno.test("an image index fails", async () => {
  const index = JSON.stringify({
    schemaVersion: 2,
    mediaType: "application/vnd.docker.distribution.manifest.v2+json",
    manifests: [],
  });
  const dig = await digestOf(index);
  const moved = loadAuthority(authorityText({
    tag: `20260716-${dig.slice(7, 15)}`,
    digest: dig,
  }));
  const { problems } = await verify(
    moved,
    RUNNER,
    serving({ digest: dig, body: encoder.encode(index) }),
  );
  assert(problems.some((line) => line.includes("image INDEX")));
});

Deno.test("a malformed or mismatched config blob fails", async () => {
  const bent = await verify(authority, RUNNER, serving({}, `${config} `));
  assert(bent.problems.some((line) => line.includes("platform fields untrusted")));
  const broken = await verify(authority, RUNNER, serving({}, "not json"));
  assert(broken.problems.some((line) => line.includes("untrusted")));
});

Deno.test("a different platform fails", async () => {
  const arm = JSON.stringify({ architecture: "arm64", os: "linux" });
  const armDigest = await digestOf(arm);
  const bentManifest = JSON.stringify({
    schemaVersion: 2,
    mediaType: "application/vnd.docker.distribution.manifest.v2+json",
    config: { digest: armDigest },
    layers: [{}],
  });
  const dig = await digestOf(bentManifest);
  const moved = loadAuthority(authorityText({
    tag: `20260716-${dig.slice(7, 15)}`,
    digest: dig,
  }));
  const bent = (url: string, _accept: string): Promise<Answer> =>
    Promise.resolve(
      url.includes("/manifests/")
        ? {
          status: 200,
          remote: "r",
          redirects: 0,
          digest: dig,
          body: encoder.encode(bentManifest),
        }
        : { status: 200, remote: "r", redirects: 0, body: encoder.encode(arm) },
    );
  const { problems } = await verify(moved, RUNNER, bent);
  assert(problems.some((line) => line.includes("platform linux/arm64 != pinned linux/amd64")));
});

Deno.test("redirects fail", async () => {
  const { problems } = await verify(authority, RUNNER, serving({ redirects: 1 }));
  assert(problems.some((line) => line.includes("redirects are refused")));
});

Deno.test("missing RUNNER_NAME fails", async () => {
  for (const runner of [undefined, ""]) {
    const { problems } = await verify(authority, runner, serving());
    assert(problems.some((line) => line.includes("CI audit contract")));
  }
});

Deno.test("initial and terminal runner-name mismatch fails", async () => {
  const text = authorityText();
  const initial = await snapshot(text, authority, "sel", "hk-01-heavy");
  const terminal = await snapshot(text, authority, "sel", "hk-04-heavy");
  const moved = unchanged(initial, terminal);
  assertEquals(moved.length, 1);
  assert(moved[0].includes("runner moved during the job"));
});

Deno.test("initial and terminal authority mutation fails", async () => {
  const initial = await snapshot(authorityText(), authority, "sel", RUNNER);
  const terminal = await snapshot(`${authorityText()} `, authority, "sel", RUNNER);
  const moved = unchanged(initial, terminal);
  assert(moved.some((line) => line.includes("authority moved during the job")));
});

Deno.test("initial and terminal selector mutation fails", async () => {
  const text = authorityText();
  const initial = await snapshot(text, authority, "mirror.perish.lan/ci/deno:a", RUNNER);
  const terminal = await snapshot(text, authority, "mirror.perish.lan/ci/deno:b", RUNNER);
  const moved = unchanged(initial, terminal);
  assert(moved.some((line) => line.includes("chosen moved during the job")));
});

Deno.test("registry error fails closed", async () => {
  const { problems } = await verify(authority, RUNNER, serving({ status: 404 }));
  assertEquals(problems, ["registry answered HTTP 404 for the versioned tag"]);
});

Deno.test("missing digest header fails closed", async () => {
  const { problems } = await verify(authority, RUNNER, serving({ digest: undefined }));
  assert(problems.some((line) => line.includes("no docker-content-digest header")));
});

Deno.test("unknown and duplicate authority fields fail", () => {
  const stray = JSON.parse(authorityText());
  stray.extra = true;
  assertThrows(() => loadAuthority(JSON.stringify(stray)), "unknown field extra");
  assertThrows(
    () => loadAuthority(JSON.stringify({ schema: "santi.ci_image.v4", registry: "r" })),
    "missing field",
  );
});

Deno.test("selector extraction reads both workflow forms", () => {
  assertEquals(
    selector("jobs:\n  guard:\n    container: mirror.perish.lan/ci/deno:20260716-2d60b029\n"),
    "mirror.perish.lan/ci/deno:20260716-2d60b029",
  );
  assertEquals(
    selector(
      "jobs:\n  guard:\n    container:\n      image: mirror.perish.lan/ci/deno:20260716-2d60b029\n",
    ),
    "mirror.perish.lan/ci/deno:20260716-2d60b029",
  );
});
