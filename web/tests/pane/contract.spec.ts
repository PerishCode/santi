import { expect, test } from "@playwright/test";
import { type Rig, sheet, stage } from "../fixture/api";

let rig: Rig;

test.afterEach(async () => {
	await rig.close();
});

test("adversarial transcript text renders as text only", async ({ page }) => {
	const hostile =
		"<img src=x onerror=\"document.title='owned'\"><b>bold</b>&amp;";
	rig = await stage({
		transcript: () =>
			sheet([
				{
					message_id: "m1",
					seq: 1,
					author: "assistant",
					text: hostile,
					at: "t",
				},
			]),
	});
	await page.goto(`http://127.0.0.1:${rig.port}/`);
	const body = page.locator(".bubble pre");
	await expect(body).toHaveText(hostile);
	expect(await body.locator("*").count()).toBe(0);
	expect(await page.title()).not.toBe("owned");
});

test("polls never overlap even when the registry answers slowly", async ({
	page,
}) => {
	rig = await stage({
		transcript: () => ({ ...sheet([], false, true), stall: 1200 }),
	});
	await page.goto(`http://127.0.0.1:${rig.port}/`);
	await page.waitForTimeout(6000);
	expect(rig.peak()).toBe(1);
	expect(rig.polls().length).toBeGreaterThan(1);
});

test("has_more pages drain immediately, then the pace resumes", async ({
	page,
}) => {
	rig = await stage({
		transcript: (count) => {
			if (count === 1) {
				return sheet(
					[
						{
							message_id: "m1",
							seq: 1,
							author: "assistant",
							text: "one",
							at: "t",
						},
					],
					true,
				);
			}
			if (count === 2) {
				return sheet(
					[
						{
							message_id: "m2",
							seq: 2,
							author: "assistant",
							text: "two",
							at: "t",
						},
					],
					false,
				);
			}
			return sheet([]);
		},
	});
	await page.goto(`http://127.0.0.1:${rig.port}/`);
	await expect(page.locator(".bubble")).toHaveCount(2);
	const polls = rig.polls();
	expect(polls[1].at - polls[0].at).toBeLessThan(1000);
	expect(polls[1].path).toContain("since=1");
});

test("a 500 retry reuses the identical client_message_id", async ({ page }) => {
	rig = await stage({
		transcript: () => sheet([], false, true),
		send: (_call, count) =>
			count === 1
				? { status: 500, body: { code: "storm" } }
				: {
						status: 200,
						body: {
							status: "accepted",
							message_id: "m1",
							client_message_id: "echo",
							receipt_id: "r1",
							received_at: "t",
						},
					},
	});
	await page.goto(`http://127.0.0.1:${rig.port}/`);
	await page.locator("textarea").fill("hello");
	await page.locator("button[type=submit]").click();
	await expect(page.locator(".bubble .note")).toContainText("rejected");
	await page.locator(".note button").click();
	await expect(page.locator(".bubble .note")).toContainText("accepted");
	const sends = rig.sends();
	expect(sends).toHaveLength(2);
	const first = JSON.parse(sends[0].body);
	const second = JSON.parse(sends[1].body);
	expect(first.client_message_id).toBe(second.client_message_id);
	expect(second.content).toBe("hello");
});

test("a network death is unknown, and retry keeps the original id", async ({
	page,
}) => {
	rig = await stage({
		transcript: () => sheet([], false, true),
		send: (_call, count) =>
			count === 1
				? { status: 0, body: null, kill: true }
				: {
						status: 200,
						body: {
							status: "accepted",
							message_id: "m1",
							client_message_id: "echo",
							receipt_id: "r1",
							received_at: "t",
						},
					},
	});
	await page.goto(`http://127.0.0.1:${rig.port}/`);
	await page.locator("textarea").fill("hello");
	await page.locator("button[type=submit]").click();
	await expect(page.locator(".bubble.unknown .note")).toContainText("unknown");
	await expect(page.locator(".bubble .note")).not.toContainText("rejected");
	await page.locator(".note button").click();
	await expect(page.locator(".bubble .note")).toContainText("accepted");
	const sends = rig.sends();
	expect(sends).toHaveLength(2);
	expect(JSON.parse(sends[0].body).client_message_id).toBe(
		JSON.parse(sends[1].body).client_message_id,
	);
});

test("a 403 freezes the composer read-only while polling continues", async ({
	page,
}) => {
	rig = await stage({
		transcript: () => sheet([], false, true),
		send: () => ({ status: 403, body: { code: "window.identity.missing" } }),
	});
	await page.goto(`http://127.0.0.1:${rig.port}/`);
	await page.locator("textarea").fill("hello");
	await page.locator("button[type=submit]").click();
	await expect(page.locator("textarea")).toBeDisabled();
	const before = rig.polls().length;
	await page.waitForTimeout(4000);
	expect(rig.polls().length).toBeGreaterThan(before);
});

test("acceptance promises nothing, silence stays silent, nothing follows up", async ({
	page,
}) => {
	rig = await stage({
		transcript: () => sheet([], false, true),
		send: () => ({
			status: 200,
			body: {
				status: "accepted",
				message_id: "m1",
				client_message_id: "echo",
				receipt_id: "r1",
				received_at: "t",
			},
		}),
	});
	await page.goto(`http://127.0.0.1:${rig.port}/`);
	await expect(page.locator("text=还没有对话")).toBeVisible();
	await page.locator("textarea").fill("hello");
	await page.locator("button[type=submit]").click();
	await expect(page.locator(".bubble .note")).toContainText("accepted");
	await page.waitForTimeout(8000);
	expect(await page.locator(".bubble").count()).toBe(1);
	expect(await page.locator(".bubble.soul").count()).toBe(0);
	expect(rig.sends()).toHaveLength(1);
	const noise = rig.calls.filter(
		(call) =>
			call.method === "POST" ||
			(!call.path.startsWith("/api/v1/window/im/transcript") &&
				call.path.startsWith("/api/") &&
				call.path !== "/api/v1/strands" &&
				call.path !== "/api/v1/window/im/send"),
	);
	expect(noise).toHaveLength(1);
});
