import type { Accepted, Author, Transcript } from "./wire";

export type State =
	| "submitting"
	| "accepted"
	| "settled"
	| "unknown"
	| "rejected";

export interface Line {
	key: string;
	id: string | null;
	author: Author;
	text: string;
	at: string | null;
	state: State | null;
	reason: string | null;
	retry: boolean;
}

export const REASONS: Record<string, string> = {
	"window.identity.missing": "身份缺失(未登录?)",
	"window.content.invalid": "内容无效",
	"window.content.oversize": "内容超长(16KiB 上限)",
	"window.rate.limited": "发送过快,稍后重试",
	"window.message.conflict": "幂等键冲突(内部错误)",
};

interface Bag {
	lines: Line[];
	since: number;
	live: boolean;
	empty: boolean;
	frozen: boolean;
	busy: boolean;
}

export class Chat {
	readonly lines: Line[];
	readonly since: number;
	readonly live: boolean;
	readonly empty: boolean;
	readonly frozen: boolean;
	readonly busy: boolean;

	constructor(bag: Bag) {
		this.lines = bag.lines;
		this.since = bag.since;
		this.live = bag.live;
		this.empty = bag.empty;
		this.frozen = bag.frozen;
		this.busy = bag.busy;
	}

	static opening(): Chat {
		return new Chat({
			lines: [],
			since: 0,
			live: false,
			empty: false,
			frozen: false,
			busy: false,
		});
	}

	polled(sheet: Transcript): Chat {
		let lines = this.lines;
		for (const entry of sheet.entries) {
			lines = fold(lines, entry);
		}
		return new Chat({
			...this,
			lines,
			since: sheet.entries.length > 0 ? sheet.next_since : this.since,
			live: true,
			empty: sheet.empty && lines.length === 0,
		});
	}

	failed(): Chat {
		return new Chat({ ...this, live: false });
	}

	drafted(key: string, text: string): Chat {
		const line: Line = {
			key,
			id: null,
			author: "human",
			text,
			at: null,
			state: "submitting",
			reason: null,
			retry: false,
		};
		return new Chat({
			...this,
			empty: false,
			busy: true,
			lines: [...this.lines, line],
		});
	}

	accepted(key: string, note: Accepted): Chat {
		return this.swap(key, {
			id: note.message_id,
			at: note.received_at,
			state: "accepted",
			reason: null,
			retry: false,
		});
	}

	lost(key: string): Chat {
		return this.swap(key, {
			state: "unknown",
			reason: "网络中断,消息可能已被接受",
			retry: true,
		});
	}

	refused(key: string, status: number, code: string | null): Chat {
		const frozen = status === 403;
		const reason = frozen
			? "此身份无发言权限"
			: (code && REASONS[code]) || code || String(status);
		const retry = !frozen && (code === "window.rate.limited" || status >= 500);
		const next = this.swap(key, { state: "rejected", reason, retry });
		return new Chat({ ...next, frozen: this.frozen || frozen });
	}

	retried(key: string): Chat {
		const next = this.swap(key, {
			state: "submitting",
			reason: null,
			retry: false,
		});
		return new Chat({ ...next, busy: true });
	}

	swap(key: string, patch: Partial<Line>): Chat {
		return new Chat({
			...this,
			busy: false,
			lines: this.lines.map((line) =>
				line.key === key ? { ...line, ...patch } : line,
			),
		});
	}
}

function fold(lines: Line[], entry: Transcript["entries"][number]): Line[] {
	const local = lines.find((line) => line.id === entry.message_id);
	if (local) {
		return lines.map((line) =>
			line.key === local.key
				? { ...line, state: "settled" as const, at: entry.at }
				: line,
		);
	}
	if (lines.some((line) => line.key === entry.message_id)) {
		return lines;
	}
	const grown = lines.slice();
	grown.push({
		key: entry.message_id,
		id: entry.message_id,
		author: entry.author,
		text: entry.text,
		at: entry.at,
		state: null,
		reason: null,
		retry: false,
	});
	return grown;
}
