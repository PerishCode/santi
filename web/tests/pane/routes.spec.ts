import { expect, test } from "@playwright/test";
import { type Rig, sheet, stage } from "../fixture/api";

let rig: Rig;

test.beforeEach(async () => {
	rig = await stage({ transcript: () => sheet([], false, true) });
});

test.afterEach(async () => {
	await rig.close();
});

test("the pane mounts at root and at the shadow prefix", async ({ page }) => {
	await page.goto(`http://127.0.0.1:${rig.port}/`);
	await expect(page.locator("h1")).toContainText("santi · window");
	await page.goto(`http://127.0.0.1:${rig.port}/panel/`);
	await expect(page.locator("h1")).toContainText("santi · window");
});

test("/panel answers 308 before any asset resolves", async ({ request }) => {
	const answer = await request.get(`http://127.0.0.1:${rig.port}/panel`, {
		maxRedirects: 0,
	});
	expect(answer.status()).toBe(308);
	expect(answer.headers().location).toBe("/panel/");
});

test("hashed assets resolve under both mounts with immutable caching", async ({
	page,
	request,
}) => {
	await page.goto(`http://127.0.0.1:${rig.port}/`);
	const src = await page.locator("script[src]").first().getAttribute("src");
	const name = String(src).replace("./", "");
	for (const mount of ["", "/panel"]) {
		const answer = await request.get(
			`http://127.0.0.1:${rig.port}${mount}/${name}`,
		);
		expect(answer.status()).toBe(200);
		expect(answer.headers()["cache-control"]).toContain("immutable");
	}
});

test("browser api requests are absolute, never under the shadow prefix", async ({
	page,
}) => {
	await page.goto(`http://127.0.0.1:${rig.port}/panel/`);
	await expect(page.locator("h1")).toContainText("santi · window");
	await page.waitForTimeout(500);
	const shadowed = rig.calls.filter((call) =>
		call.path.startsWith("/panel/api"),
	);
	expect(shadowed).toHaveLength(0);
	expect(rig.polls().length).toBeGreaterThan(0);
});

test("pathname fallback does not exist at either mount", async ({
	request,
}) => {
	for (const path of ["/one/two", "/panel/one/two"]) {
		const answer = await request.get(`http://127.0.0.1:${rig.port}${path}`);
		expect(answer.status()).toBe(404);
	}
});

test("unknown api paths answer json 404", async ({ request }) => {
	const answer = await request.get(`http://127.0.0.1:${rig.port}/api/nowhere`);
	expect(answer.status()).toBe(404);
	expect(await answer.json()).toEqual({ error: "unknown api path" });
});
