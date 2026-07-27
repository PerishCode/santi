import { inspectLeaves } from "@/lib/word/leaves.ts";

function assertEquals(actual: unknown, expected: unknown): void {
  const left = JSON.stringify(actual);
  const right = JSON.stringify(expected);
  if (left !== right) {
    throw new Error(`values differ: actual ${left}, expected ${right}`);
  }
}

Deno.test("single-word test leaves pass", () => {
  const source = `
#[test]
fn passes() {}

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn awaits2() {}
`;
  assertEquals(inspectLeaves(source, "fixture.rs"), []);
});

Deno.test("compound attributed leaves fail with their source location", () => {
  const source = `
fn helper_name() {}

#[test]
fn two_words() {}
`;
  assertEquals(inspectLeaves(source, "fixture.rs"), [
    { path: "fixture.rs", line: 5, name: "two_words" },
  ]);
});

Deno.test("compound helpers remain outside the constraint", () => {
  const source = `
fn helper_name() {}
// #[test]
// fn commented_out() {}
`;
  assertEquals(inspectLeaves(source, "fixture.rs"), []);
});
