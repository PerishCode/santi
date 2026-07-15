//! Golden vectors for the wire-seat verifier (D2, web re-platform).

import { parseSeat, Seat, spellings, sweep, verifySeat } from "@/lib/word/seat.ts";

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

const seat: Seat = {
  schema: "santi.wire_seat.v1",
  file: "web/src/lib/wire.ts",
  types: {
    Send: "WindowSendRequest",
    Author: "WindowAuthor",
  },
};

const schemas: Record<string, Record<string, unknown>> = {
  WindowSendRequest: {
    type: "object",
    required: ["content", "client_message_id"],
    properties: {
      content: { type: "string" },
      client_message_id: { type: "string" },
      cursor: { type: ["integer", "null"] },
      at: { $ref: "#/components/schemas/String" },
      author: { $ref: "#/components/schemas/WindowAuthor" },
    },
  },
  WindowAuthor: { type: "string", enum: ["human", "assistant"] },
  String: { type: "string" },
};

const lawful = `export type Author = "human" | "assistant";

export type Send = {
	content: string;
	client_message_id: string;
	cursor?: number | null;
	at?: string;
	author?: Author;
};
`;

Deno.test("lawful seat verifies clean", () => {
  assertEquals(verifySeat(lawful, seat, schemas), []);
});

Deno.test("parser rejects leftovers", () => {
  const { problems } = parseSeat(`${lawful}\nconst stray = 1;\n`);
  assertEquals(problems.length, 1);
  assert(problems[0].includes("unsupported content"));
});

Deno.test("unmapped export fails", () => {
  const extra = `${lawful}\nexport type Ghost = {\n\tcontent: string;\n};\n`;
  const problems = verifySeat(extra, seat, schemas);
  assert(problems.some((entry) => entry.includes("Ghost is not mapped")));
});

Deno.test("undeclared mapped type fails", () => {
  const problems = verifySeat(`export type Author = "human" | "assistant";`, seat, schemas);
  assert(problems.some((entry) => entry.includes("Send is not declared")));
});

Deno.test("non-injective map fails", () => {
  const doubled: Seat = {
    ...seat,
    types: { Send: "WindowSendRequest", Again: "WindowSendRequest" },
  };
  const source =
    `export type Send = {\n\tcontent: string;\n};\nexport type Again = {\n\tcontent: string;\n};\n`;
  const problems = verifySeat(source, doubled, schemas);
  assert(problems.some((entry) => entry.includes("not injective")));
});

Deno.test("absent component fails", () => {
  const ghost: Seat = { ...seat, types: { Send: "Nowhere" } };
  const source = `export type Send = {\n\tcontent: string;\n};\n`;
  const problems = verifySeat(source, ghost, schemas);
  assert(problems.some((entry) => entry.includes("absent from the verified schema")));
});

Deno.test("missing schema property fails", () => {
  const source = lawful.replace("\tcontent: string;\n", "");
  const problems = verifySeat(source, seat, schemas);
  assert(problems.some((entry) => entry.includes("WindowSendRequest.content is missing")));
});

Deno.test("undeclared extra property fails", () => {
  const source = lawful.replace("\tcontent: string;", "\tcontent: string;\n\tstray: string;");
  const problems = verifySeat(source, seat, schemas);
  assert(problems.some((entry) => entry.includes("undeclared extra")));
});

Deno.test("requiredness flip fails", () => {
  const source = lawful.replace("content: string;", "content?: string;");
  const problems = verifySeat(source, seat, schemas);
  assert(problems.some((entry) => entry.includes("required by schema but optional")));
});

Deno.test("optionality flip fails", () => {
  const source = lawful.replace("cursor?: number | null;", "cursor: number | null;");
  const problems = verifySeat(source, seat, schemas);
  assert(problems.some((entry) => entry.includes("optional in schema but required")));
});

Deno.test("nullability mismatch fails", () => {
  const source = lawful.replace("cursor?: number | null;", "cursor?: number;");
  const problems = verifySeat(source, seat, schemas);
  assert(problems.some((entry) => entry.includes('!= schema shape "number | null"')));
});

Deno.test("enum mismatch fails", () => {
  const source = lawful.replace('"human" | "assistant"', '"human" | "machine"');
  const problems = verifySeat(source, seat, schemas);
  assert(problems.some((entry) => entry.includes("enum")));
});

Deno.test("unsupported construct fails", () => {
  const twisted = structuredClone(schemas);
  (twisted.WindowSendRequest.properties as Record<string, unknown>).content = {
    allOf: [{ type: "string" }],
  };
  const problems = verifySeat(lawful, seat, twisted);
  assert(problems.some((entry) => entry.includes("Send.content")));
});

Deno.test("spellings collect snake properties only", () => {
  assertEquals(spellings(seat, schemas), ["client_message_id"]);
});

Deno.test("declaration under web/tests fails with a named row", () => {
  const files = [
    {
      path: "web/tests/fixture/shape.ts",
      text: "type Copy = {\n\tclient_message_id: string;\n};\n",
    },
  ];
  const faults = sweep(files, seat, ["client_message_id"]);
  assertEquals(faults.length, 1);
  assert(faults[0].includes("web/tests/fixture/shape.ts:2"));
});

Deno.test("interface and class declarations fail too", () => {
  const files = [
    {
      path: "web/src/lib/copy.ts",
      text: "interface Shadow {\n\treadonly client_message_id: string;\n}\n" +
        "class Holder {\n\tclient_message_id?: string;\n}\n",
    },
  ];
  const faults = sweep(files, seat, ["client_message_id"]);
  assertEquals(faults.length, 2);
});

Deno.test("fixture object and JSON uses are allowed", () => {
  const files = [
    {
      path: "web/tests/fixture/api.ts",
      text: 'const body = {\n\tclient_message_id: "m1",\n};\n' +
        'const raw = \'{"client_message_id":"m1"}\';\n' +
        "const key = body.client_message_id;\n" +
        'expect(sent.client_message_id).toBe("m1");\n',
    },
  ];
  assertEquals(sweep(files, seat, ["client_message_id"]), []);
});

Deno.test("imported seat types are allowed", () => {
  const files = [
    {
      path: "web/tests/route.test.ts",
      text: 'import type { Send } from "../src/lib/wire";\n' +
        'const message: Send = { content: "hi", client_message_id: "m1" };\n',
    },
  ];
  assertEquals(sweep(files, seat, ["client_message_id"]), []);
});

Deno.test("the seat file itself is exempt from the sweep", () => {
  const files = [
    {
      path: "web/src/lib/wire.ts",
      text: "export type Send = {\n\tclient_message_id: string;\n};\n",
    },
  ];
  assertEquals(sweep(files, seat, ["client_message_id"]), []);
});
