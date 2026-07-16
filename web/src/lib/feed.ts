export type Raw = Record<string, unknown>;

export const NAMES = [
	"stream_open",
	"message_created",
	"message_delta",
	"message_completed",
	"tool_call_created",
	"tool_result_created",
	"thinking_created",
	"thinking_updated",
	"thinking_completed",
	"material_updated",
	"turn_started",
	"turn_activity",
	"turn_completed",
	"turn_failed",
];

export interface Occ {
	key: string;
	kind: string;
	label: string;
	at: string | null;
	body: string;
}

export function grab(raw: unknown, name: string): unknown {
	if (raw === null || typeof raw !== "object") {
		return undefined;
	}
	return (raw as Raw)[name];
}

export function text(raw: unknown, name: string): string {
	const value = grab(raw, name);
	return typeof value === "string" ? value : "";
}

function show(value: unknown): string {
	if (value === null || value === undefined) {
		return "";
	}
	return typeof value === "string" ? value : JSON.stringify(value, null, 2);
}

interface Bag {
	occs: Occ[];
	turn: string | null;
	live: boolean;
	marks: number;
}

export class Feed {
	readonly occs: Occ[];
	readonly turn: string | null;
	readonly live: boolean;
	readonly marks: number;

	constructor(bag: Bag) {
		this.occs = bag.occs;
		this.turn = bag.turn;
		this.live = bag.live;
		this.marks = bag.marks;
	}

	static opening(): Feed {
		return new Feed({ occs: [], turn: null, live: false, marks: 0 });
	}

	replayed(snap: unknown): Feed {
		const staged: Array<{ at: string; seq: number; occ: Occ }> = [];
		for (const raw of roster(snap, "messages")) {
			const occ = message(raw);
			if (occ !== null) {
				const seq = Number(grab(grab(raw, "relation"), "strand_seq") ?? 0);
				staged.push({ at: occ.at ?? "", seq, occ });
			}
		}
		for (const raw of roster(snap, "thinking_spans")) {
			staged.push({ at: text(raw, "created_at"), seq: 0, occ: thinking(raw) });
		}
		for (const raw of roster(snap, "tool_calls")) {
			staged.push({ at: text(raw, "created_at"), seq: 0, occ: call(raw) });
		}
		for (const raw of roster(snap, "tool_results")) {
			staged.push({ at: text(raw, "created_at"), seq: 0, occ: outcome(raw) });
		}
		for (const raw of roster(snap, "compacts")) {
			staged.push({ at: "", seq: -1, occ: compact(raw) });
		}
		staged.sort((a, b) => a.at.localeCompare(b.at) || a.seq - b.seq);
		let feed = new Feed({ ...this, occs: [] });
		for (const item of staged) {
			feed = feed.upsert(item.occ);
		}
		return feed;
	}

	handled(env: unknown): Feed {
		const raw = (grab(env, "payload") ?? env) as Raw;
		const deed = DEEDS[text(raw, "type")];
		return deed === undefined ? this : deed(this, raw, env);
	}

	opened(): Feed {
		return new Feed({ ...this, live: true });
	}

	failed(): Feed {
		return new Feed({ ...this, live: false });
	}

	upsert(occ: Occ | null): Feed {
		if (occ === null) {
			return this;
		}
		const known = this.occs.some((row) => row.key === occ.key);
		const occs = known
			? this.occs.map((row) => (row.key === occ.key ? occ : row))
			: [...this.occs, occ];
		return new Feed({ ...this, occs });
	}

	drop(key: string): Feed {
		return new Feed({
			...this,
			occs: this.occs.filter((row) => row.key !== key),
		});
	}

	acting(turn: string | null): Feed {
		return new Feed({ ...this, turn });
	}

	marked(occ: Occ): Feed {
		const next = this.upsert({ ...occ, key: `${occ.key}:${this.marks}` });
		return new Feed({ ...next, marks: this.marks + 1 });
	}
}

function roster(raw: unknown, name: string): Raw[] {
	const value = grab(raw, name);
	return Array.isArray(value) ? (value as Raw[]) : [];
}

function message(note: unknown): Occ | null {
	const inner = grab(note, "message");
	const id = text(inner, "id");
	if (id === "") {
		return null;
	}
	const kind = text(inner, "actor_type") === "system" ? "system" : "soul";
	const at =
		text(grab(note, "relation"), "created_at") || text(inner, "created_at");
	return {
		key: `msg:${id}`,
		kind,
		label: kind,
		at: at || null,
		body: text(note, "content_text"),
	};
}

function thinking(span: unknown): Occ {
	const running = text(span, "state") === "running";
	const fallback = running ? "(思考中…)" : "(无摘要)";
	return {
		key: `think:${text(span, "id")}`,
		kind: "thinking",
		label: running ? "thinking · 进行中" : "thinking · 完成",
		at: text(span, "created_at") || null,
		body: text(span, "summary") || fallback,
	};
}

function call(tool: unknown): Occ {
	return {
		key: `tool:${text(tool, "id")}`,
		kind: "tool",
		label: `tool · ${text(tool, "tool_name")}`,
		at: text(tool, "created_at") || null,
		body: show(grab(tool, "arguments")),
	};
}

function outcome(result: unknown): Occ {
	const error = text(result, "error_text");
	return {
		key: `result:${text(result, "id")}`,
		kind: "result",
		label: "tool result",
		at: text(result, "created_at") || null,
		body: error !== "" ? `ERROR: ${error}` : show(grab(result, "output")),
	};
}

function compact(record: unknown): Occ {
	return {
		key: `compact:${text(record, "id")}`,
		kind: "compact",
		label: "compact",
		at: null,
		body: text(record, "summary"),
	};
}

type Deed = (feed: Feed, raw: Raw, env: unknown) => Feed;

const DEEDS: Record<string, Deed> = {
	message_created: (feed, raw) => feed.upsert(message(grab(raw, "message"))),
	message_delta: (feed, raw, env) => {
		const key = `live:${text(raw, "turn_id")}`;
		const before = feed.occs.find((row) => row.key === key);
		const kind = text(raw, "role") === "system" ? "system" : "soul";
		return feed.upsert({
			key,
			kind,
			label: `${kind} · …`,
			at: text(env, "created_at") || null,
			body: (before?.body ?? "") + text(raw, "text"),
		});
	},
	message_completed: (feed, raw) =>
		feed
			.drop(`live:${text(raw, "turn_id")}`)
			.upsert(message(grab(raw, "message"))),
	thinking_created: (feed, raw) => feed.upsert(thinking(grab(raw, "thinking"))),
	thinking_updated: (feed, raw) => feed.upsert(thinking(grab(raw, "thinking"))),
	thinking_completed: (feed, raw) =>
		feed.upsert(thinking(grab(raw, "thinking"))),
	tool_call_created: (feed, raw) => feed.upsert(call(grab(raw, "tool_call"))),
	tool_result_created: (feed, raw) =>
		feed.upsert(outcome(grab(raw, "tool_result"))),
	turn_started: (feed) => feed.acting("turn…"),
	turn_activity: (feed, raw) =>
		feed.acting(text(grab(raw, "activity"), "state") || "active"),
	turn_completed: (feed, _raw, env) =>
		feed.acting(null).marked({
			key: "turn",
			kind: "turn",
			label: "turn completed",
			at: text(env, "created_at") || null,
			body: "",
		}),
	turn_failed: (feed, raw, env) =>
		feed.acting(null).marked({
			key: "fail",
			kind: "fail",
			label: "turn failed",
			at: text(env, "created_at") || null,
			body: text(raw, "error"),
		}),
};
