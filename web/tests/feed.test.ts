import { expect, test } from "vitest";
import { Feed } from "../src/lib/feed";

const snap = {
	messages: [
		{
			message: {
				id: "m1",
				actor_type: "soul",
				created_at: "2026-01-01T00:00:02Z",
			},
			relation: { created_at: "2026-01-01T00:00:02Z", strand_seq: 2 },
			content_text: "hello",
		},
		{
			message: {
				id: "m0",
				actor_type: "system",
				created_at: "2026-01-01T00:00:01Z",
			},
			relation: { created_at: "2026-01-01T00:00:01Z", strand_seq: 1 },
			content_text: "seed",
		},
	],
	thinking_spans: [
		{
			id: "t1",
			state: "completed",
			summary: "pondered",
			created_at: "2026-01-01T00:00:03Z",
		},
	],
	tool_calls: [
		{
			id: "c1",
			tool_name: "shell",
			arguments: { cmd: "ls" },
			created_at: "2026-01-01T00:00:04Z",
		},
	],
	tool_results: [
		{ id: "r1", output: "ok", created_at: "2026-01-01T00:00:05Z" },
	],
	compacts: [{ id: "k1", summary: "folded" }],
};

test("replay orders occurrences chronologically with compacts first", () => {
	const feed = Feed.opening().replayed(snap);
	expect(feed.occs.map((occ) => occ.key)).toEqual([
		"compact:k1",
		"msg:m0",
		"msg:m1",
		"think:t1",
		"tool:c1",
		"result:r1",
	]);
	expect(feed.occs[1].kind).toBe("system");
	expect(feed.occs[2].body).toBe("hello");
});

test("deltas accumulate under a provisional key and resolve on completion", () => {
	let feed = Feed.opening();
	feed = feed.handled({
		created_at: "t",
		payload: { type: "message_delta", turn_id: "tn", text: "he" },
	});
	feed = feed.handled({
		created_at: "t",
		payload: { type: "message_delta", turn_id: "tn", text: "llo" },
	});
	expect(feed.occs).toHaveLength(1);
	expect(feed.occs[0].body).toBe("hello");
	feed = feed.handled({
		payload: {
			type: "message_completed",
			turn_id: "tn",
			message: {
				message: { id: "m9", actor_type: "soul", created_at: "t2" },
				content_text: "hello!",
			},
		},
	});
	expect(feed.occs).toHaveLength(1);
	expect(feed.occs[0].key).toBe("msg:m9");
	expect(feed.occs[0].body).toBe("hello!");
});

test("thinking updates land in place", () => {
	let feed = Feed.opening();
	feed = feed.handled({
		payload: {
			type: "thinking_created",
			thinking: { id: "t1", state: "running" },
		},
	});
	expect(feed.occs[0].body).toBe("(思考中…)");
	feed = feed.handled({
		payload: {
			type: "thinking_completed",
			thinking: { id: "t1", state: "completed", summary: "done" },
		},
	});
	expect(feed.occs).toHaveLength(1);
	expect(feed.occs[0].body).toBe("done");
});

test("turn lifecycle drives the activity pill and marks", () => {
	let feed = Feed.opening();
	feed = feed.handled({ payload: { type: "turn_started" } });
	expect(feed.turn).toBe("turn…");
	feed = feed.handled({
		payload: { type: "turn_activity", activity: { state: "driving" } },
	});
	expect(feed.turn).toBe("driving");
	feed = feed.handled({ created_at: "t", payload: { type: "turn_completed" } });
	expect(feed.turn).toBeNull();
	feed = feed.handled({
		created_at: "t",
		payload: { type: "turn_failed", error: "boom" },
	});
	expect(feed.occs.filter((occ) => occ.kind === "turn")).toHaveLength(1);
	expect(feed.occs.filter((occ) => occ.kind === "fail")).toHaveLength(1);
});

test("unknown events are ignored", () => {
	const feed = Feed.opening().handled({
		payload: { type: "material_updated" },
	});
	expect(feed.occs).toHaveLength(0);
});

test("tool errors render loudly", () => {
	const feed = Feed.opening().handled({
		payload: {
			type: "tool_result_created",
			tool_result: { id: "r1", error_text: "denied" },
		},
	});
	expect(feed.occs[0].body).toBe("ERROR: denied");
});
