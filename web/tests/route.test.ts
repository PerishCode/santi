import { expect, test } from "vitest";
import { resolve, table } from "../src/lib/route";
import { Window } from "../src/views/Window";

test("empty", () => {
	expect(resolve("")).toBe(Window);
});

test("root", () => {
	expect(resolve("#/")).toBe(Window);
});

test("unknown", () => {
	expect(resolve("#/nowhere")).toBe(Window);
});

test("table", () => {
	for (const view of Object.values(table)) {
		expect(typeof view).toBe("function");
	}
});
