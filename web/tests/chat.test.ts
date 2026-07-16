import { expect, test } from "vitest";
import { Chat } from "../src/lib/chat";
import type { Transcript } from "../src/lib/wire";

function sheet(
	entries: Transcript["entries"],
	more = false,
	none = false,
): Transcript {
	return {
		participant: "window:abc",
		entries,
		next_since: entries.length > 0 ? entries[entries.length - 1].seq : 0,
		has_more: more,
		empty: none,
	};
}

test("transcript entries append once and advance the cursor", () => {
	const entry = {
		message_id: "m1",
		seq: 4,
		author: "assistant" as const,
		text: "hi",
		at: "t1",
	};
	let face = Chat.opening().polled(sheet([entry]));
	face = face.polled(sheet([entry]));
	expect(face.lines).toHaveLength(1);
	expect(face.since).toBe(4);
	expect(face.live).toBe(true);
});

test("the cursor never advances on an empty page", () => {
	let face = Chat.opening().polled(
		sheet([
			{
				message_id: "m1",
				seq: 9,
				author: "human" as const,
				text: "x",
				at: "t",
			},
		]),
	);
	face = face.polled(sheet([]));
	expect(face.since).toBe(9);
});

test("silence is legitimate, not an error", () => {
	const face = Chat.opening().polled(sheet([], false, true));
	expect(face.empty).toBe(true);
	expect(face.live).toBe(true);
	expect(face.lines).toHaveLength(0);
});

test("a poll failure goes offline without inventing content", () => {
	const face = Chat.opening().polled(sheet([])).failed();
	expect(face.live).toBe(false);
	expect(face.lines).toHaveLength(0);
});

test("an accepted message settles when the transcript echoes it", () => {
	let face = Chat.opening().drafted("k1", "hello");
	face = face.accepted("k1", {
		status: "accepted",
		message_id: "m9",
		client_message_id: "k1",
		receipt_id: "r1",
		received_at: "t9",
	});
	expect(face.lines[0].state).toBe("accepted");
	face = face.polled(
		sheet([
			{ message_id: "m9", seq: 1, author: "human", text: "hello", at: "t9" },
		]),
	);
	expect(face.lines).toHaveLength(1);
	expect(face.lines[0].state).toBe("settled");
});

test("a network death is unknown, never failed", () => {
	let face = Chat.opening().drafted("k1", "hello");
	face = face.lost("k1");
	expect(face.lines[0].state).toBe("unknown");
	expect(face.lines[0].retry).toBe(true);
});

test("retry keeps the original key and returns to submitting", () => {
	let face = Chat.opening().drafted("k1", "hello");
	face = face.lost("k1");
	face = face.retried("k1");
	expect(face.lines[0].key).toBe("k1");
	expect(face.lines[0].state).toBe("submitting");
	expect(face.lines[0].retry).toBe(false);
});

test("a 403 freezes the composer into read-only", () => {
	let face = Chat.opening().drafted("k1", "hello");
	face = face.refused("k1", 403, null);
	expect(face.frozen).toBe(true);
	expect(face.lines[0].state).toBe("rejected");
});

test("rate limits and server errors offer retry; validation does not", () => {
	let face = Chat.opening().drafted("k1", "a");
	face = face.refused("k1", 429, "window.rate.limited");
	expect(face.lines[0].retry).toBe(true);
	let other = Chat.opening().drafted("k2", "b");
	other = other.refused("k2", 400, "window.content.invalid");
	expect(other.lines[0].retry).toBe(false);
	expect(other.frozen).toBe(false);
});
