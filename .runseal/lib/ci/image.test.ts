//! Address-bound registry-drift gate vectors (Liberte CI-image amendment v3).

import {
  agree,
  Answer,
  loadAuthority,
  mapping,
  member,
  Selection,
  selector,
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

const one = JSON.stringify({
  schemaVersion: 2,
  mediaType: "application/vnd.docker.distribution.manifest.v2+json",
  config: { digest: configDigest },
  layers: [{}],
});
const four = JSON.stringify({
  schemaVersion: 2,
  mediaType: "application/vnd.docker.distribution.manifest.v2+json",
  config: { digest: configDigest },
  layers: [{}, {}],
});

const oneDigest = await digestOf(one);
const fourDigest = await digestOf(four);

const CANONICAL = "sha256:1cb896ad03bbc246329efe714ce103e4be6c9580e17561e97664034382423630";
const HYPHENATED = "sha256:1cb896ad0-3bbc246329efe714ce103e4be6c9580e17561e97664034382423630";

function authorityText(members: Record<string, { address: string; digest: string }>): string {
  return JSON.stringify({
    schema: "santi.ci_image.v3",
    registry: "mirror.perish.lan",
    repository: "ci/deno",
    tag: "latest",
    media: "application/vnd.docker.distribution.manifest.v2+json",
    platform: "linux/amd64",
    members,
  });
}

const authority = loadAuthority(authorityText({
  "hk-01-heavy": { address: "43.251.225.113", digest: oneDigest },
  "hk-04-heavy": { address: "38.76.202.208", digest: fourDigest },
}));

const HK01: Selection = { label: "hk-01-heavy", address: "43.251.225.113" };
const HK04: Selection = { label: "hk-04-heavy", address: "38.76.202.208" };

function serving(manifest: string, digest: string, overrides: Partial<Answer> = {}) {
  return (url: string, _accept: string, address: string): Promise<Answer> => {
    const base: Answer = url.includes("/manifests/")
      ? { status: 200, remote: address, redirects: 0, digest, body: encoder.encode(manifest) }
      : { status: 200, remote: address, redirects: 0, body: encoder.encode(config) };
    return Promise.resolve(url.includes("/manifests/") ? { ...base, ...overrides } : base);
  };
}

function counting(inner: ReturnType<typeof serving>) {
  let calls = 0;
  const client = (url: string, accept: string, address: string) => {
    calls += 1;
    return inner(url, accept, address);
  };
  return { client, count: () => calls };
}

Deno.test("hk-01 address with hk-01 digest passes", async () => {
  const { problems, evidence } = await verify(authority, HK01, serving(one, oneDigest));
  assertEquals(problems, []);
  assert(evidence.some((line) => line.includes("selected member hk-01-heavy at 43.251.225.113")));
  assert(evidence.some((line) => line.includes("observed platform linux/amd64")));
});

Deno.test("hk-04 address with hk-04 digest passes", async () => {
  const { problems } = await verify(authority, HK04, serving(four, fourDigest));
  assertEquals(problems, []);
});

Deno.test("hk-01 address observing hk-04's approved digest fails", async () => {
  const { problems } = await verify(authority, HK01, serving(four, fourDigest));
  assert(problems.some((line) => line.includes(`header digest ${fourDigest} != member digest`)));
  assert(problems.some((line) => line.includes("independently hashed manifest")));
});

Deno.test("hk-04 address observing hk-01's approved digest fails", async () => {
  const { problems } = await verify(authority, HK04, serving(one, oneDigest));
  assert(problems.some((line) => line.includes(`header digest ${oneDigest} != member digest`)));
});

Deno.test("missing hosts entry fails before network access", () => {
  const hosts = "127.0.0.1 localhost\n::1 localhost\n";
  const mapped = mapping(hosts, "mirror.perish.lan");
  assert(mapped.problem !== undefined && mapped.problem.includes("no entry"));
});

Deno.test("unknown address fails before network access", async () => {
  const found = member(authority, "10.0.0.9");
  assert(found.problem !== undefined && found.problem.includes("not a reviewed member"));
  const spy = counting(serving(one, oneDigest));
  if (found.problem !== undefined) {
    assertEquals(spy.count(), 0, "no registry request may be attempted");
  }
  await Promise.resolve();
});

Deno.test("multiple different addresses fail", () => {
  const hosts = "43.251.225.113 mirror.perish.lan\n38.76.202.208 mirror.perish.lan\n";
  const mapped = mapping(hosts, "mirror.perish.lan");
  assert(mapped.problem !== undefined && mapped.problem.includes("2 lines"));
});

Deno.test("duplicate identical entries fail rather than collapse", () => {
  const hosts = "43.251.225.113 mirror.perish.lan\n43.251.225.113 mirror.perish.lan\n";
  const mapped = mapping(hosts, "mirror.perish.lan");
  assert(mapped.problem !== undefined && mapped.problem.includes("never collapsed"));
});

Deno.test("duplicate authority addresses fail", () => {
  assertThrows(
    () =>
      loadAuthority(authorityText({
        "hk-01-heavy": { address: "43.251.225.113", digest: oneDigest },
        "hk-04-heavy": { address: "43.251.225.113", digest: fourDigest },
      })),
    "duplicate member address",
  );
});

Deno.test("malformed hosts address fails", () => {
  const mapped = mapping("999.1.2.3 mirror.perish.lan\n", "mirror.perish.lan");
  assert(mapped.problem !== undefined && mapped.problem.includes("malformed address"));
});

Deno.test("malformed authority address fails", () => {
  assertThrows(
    () =>
      loadAuthority(authorityText({
        "hk-01-heavy": { address: "not-an-ip", digest: oneDigest },
      })),
    "malformed address for member hk-01-heavy",
  );
});

Deno.test("exact hostname tokens ignore lookalike suffixes", () => {
  const hosts = [
    "1.2.3.4 notmirror.perish.lan",
    "5.6.7.8 mirror.perish.lan.evil",
    "43.251.225.113 mirror.perish.lan",
  ].join("\n");
  assertEquals(mapping(hosts, "mirror.perish.lan"), { address: "43.251.225.113" });
});

Deno.test("comments and whitespace parse correctly", () => {
  const hosts = [
    "# managed by the runner",
    "",
    "   43.251.225.113\tmirror.perish.lan   # appliance closed loop",
    "127.0.0.1 localhost",
  ].join("\n");
  assertEquals(mapping(hosts, "mirror.perish.lan"), { address: "43.251.225.113" });
});

Deno.test("effective peer mismatch fails", async () => {
  const { problems } = await verify(
    authority,
    HK01,
    serving(one, oneDigest, { remote: "9.9.9.9" }),
  );
  assertEquals(problems, ["effective peer 9.9.9.9 != selected address 43.251.225.113"]);
});

Deno.test("a redirect fails", async () => {
  const { problems } = await verify(
    authority,
    HK01,
    serving(one, oneDigest, { redirects: 1 }),
  );
  assert(problems.some((line) => line.includes("redirects are refused")));
});

Deno.test("initial and terminal member mismatch fails", () => {
  assertEquals(unchanged(HK01, HK01), []);
  const moved = unchanged(HK01, HK04);
  assertEquals(moved.length, 1);
  assert(moved[0].includes("selection moved during the job"));
});

Deno.test("canonical hk-04 digest loads while the hyphenated transcription fails", () => {
  loadAuthority(authorityText({
    "hk-04-heavy": { address: "38.76.202.208", digest: CANONICAL },
  }));
  assertThrows(
    () =>
      loadAuthority(authorityText({
        "hk-04-heavy": { address: "38.76.202.208", digest: HYPHENATED },
      })),
    "malformed digest for member hk-04-heavy",
  );
});

Deno.test("config blob hash must match the manifest-named digest", async () => {
  const bent = (url: string, _accept: string, address: string): Promise<Answer> =>
    Promise.resolve(
      url.includes("/manifests/")
        ? {
          status: 200,
          remote: address,
          redirects: 0,
          digest: oneDigest,
          body: encoder.encode(one),
        }
        : {
          status: 200,
          remote: address,
          redirects: 0,
          body: encoder.encode(`${config} `),
        },
    );
  const { problems } = await verify(authority, HK01, bent);
  assert(problems.some((line) => line.includes("platform fields untrusted")));
});

Deno.test("empty member map fails closed", () => {
  assertThrows(() => loadAuthority(authorityText({})), "member map is missing or empty");
});

Deno.test("unknown authority field fails closed", () => {
  const stray = JSON.parse(
    authorityText({ "hk-01-heavy": { address: "43.251.225.113", digest: oneDigest } }),
  );
  stray.extra = true;
  assertThrows(() => loadAuthority(JSON.stringify(stray)), "unknown field extra");
});

Deno.test("duplicate member identity fails closed", () => {
  const doubled = authorityText({
    "hk-01-heavy": { address: "43.251.225.113", digest: oneDigest },
  }).replace(
    `"hk-01-heavy":`,
    `"hk-01-heavy":{"address":"38.76.202.208","digest":"${fourDigest}"},"hk-01-heavy":`,
  );
  assertThrows(() => loadAuthority(doubled), "duplicate member identity hk-01-heavy");
});

Deno.test("selector mismatch fails", () => {
  assertEquals(agree(authority, "mirror.perish.lan/ci/deno").length, 0);
  assertEquals(agree(authority, "mirror.perish.lan/ci/deno:latest").length, 0);
  assert(agree(authority, "mirror.perish.lan/ci/rust").length === 1);
  assert(agree(authority, "mirror.perish.lan/ci/deno:stable").length === 1);
  assert(agree(authority, "docker.io/ci/deno").length === 1);
});

Deno.test("missing digest header fails closed", async () => {
  const { problems } = await verify(
    authority,
    HK01,
    serving(one, oneDigest, { digest: undefined }),
  );
  assert(problems.some((line) => line.includes("no docker-content-digest header")));
});

Deno.test("malformed manifest fails closed", async () => {
  const { problems } = await verify(
    authority,
    HK01,
    serving(one, oneDigest, { body: encoder.encode("not json") }),
  );
  assert(problems.some((line) => line.includes("not valid JSON")));
});

Deno.test("registry error fails closed", async () => {
  const { problems } = await verify(
    authority,
    HK01,
    serving(one, oneDigest, { status: 503 }),
  );
  assertEquals(problems, ["registry answered HTTP 503 for the manifest"]);
});

Deno.test("image index is refused outright", async () => {
  const index = JSON.stringify({
    schemaVersion: 2,
    mediaType: "application/vnd.docker.distribution.manifest.v2+json",
    manifests: [],
  });
  const moved = loadAuthority(authorityText({
    "hk-01-heavy": { address: "43.251.225.113", digest: await digestOf(index) },
  }));
  const { problems } = await verify(moved, HK01, serving(index, await digestOf(index)));
  assert(problems.some((line) => line.includes("image INDEX")));
});

Deno.test("platform mismatch fails", async () => {
  const arm = JSON.stringify({ architecture: "arm64", os: "linux" });
  const armDigest = await digestOf(arm);
  const bentManifest = JSON.stringify({
    schemaVersion: 2,
    mediaType: "application/vnd.docker.distribution.manifest.v2+json",
    config: { digest: armDigest },
    layers: [{}],
  });
  const moved = loadAuthority(authorityText({
    "hk-01-heavy": { address: "43.251.225.113", digest: await digestOf(bentManifest) },
  }));
  const bent = (url: string, _accept: string, address: string): Promise<Answer> =>
    Promise.resolve(
      url.includes("/manifests/")
        ? {
          status: 200,
          remote: address,
          redirects: 0,
          digest: moved.members["hk-01-heavy"].digest,
          body: encoder.encode(bentManifest),
        }
        : { status: 200, remote: address, redirects: 0, body: encoder.encode(arm) },
    );
  const { problems } = await verify(moved, HK01, bent);
  assert(problems.some((line) => line.includes("platform linux/arm64 != pinned linux/amd64")));
});

Deno.test("selector extraction reads both workflow forms", () => {
  assertEquals(
    selector("jobs:\n  guard:\n    container: mirror.perish.lan/ci/deno\n"),
    "mirror.perish.lan/ci/deno",
  );
  assertEquals(
    selector("jobs:\n  guard:\n    container:\n      image: mirror.perish.lan/ci/deno:latest\n"),
    "mirror.perish.lan/ci/deno:latest",
  );
});

Deno.test("malformed authority fails closed", () => {
  assertThrows(
    () => loadAuthority(JSON.stringify({ schema: "santi.ci_image.v3", registry: "r" })),
    "missing field",
  );
});
