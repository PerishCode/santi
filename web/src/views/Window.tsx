import { type MutableRefObject, useEffect, useRef, useState } from "react";
import { Bubble } from "../components/Bubble/Bubble";
import { Composer } from "../components/Composer/Composer";
import { Deck } from "../components/Deck/Deck";
import { Pill } from "../components/Pill/Pill";
import { Reel } from "../components/Reel/Reel";
import { Roll } from "../components/Roll/Roll";
import { Chat, type Line } from "../lib/chat";
import { Feed, NAMES, type Raw, text } from "../lib/feed";
import type { Accepted, Transcript } from "../lib/wire";

const PACE = 3500;

type Shape = (turn: (now: Chat) => Chat) => void;
type Mold = (turn: (now: Feed) => Feed) => void;
type Gone = MutableRefObject<boolean>;
type Cursor = MutableRefObject<number>;

function pause(ms: number): Promise<void> {
	return new Promise((settle) => setTimeout(settle, ms));
}

async function pull(
	cursor: Cursor,
	gone: Gone,
	shape: Shape,
): Promise<boolean> {
	const answer = await fetch(
		`/api/v1/window/im/transcript?since=${cursor.current}`,
		{
			headers: { Accept: "application/json" },
		},
	);
	if (!answer.ok) {
		throw new Error(String(answer.status));
	}
	const sheet: Transcript = await answer.json();
	if (gone.current) {
		return false;
	}
	shape((now) => now.polled(sheet));
	if (sheet.entries.length > 0) {
		cursor.current = sheet.next_since;
	}
	return sheet.has_more;
}

async function drain(cursor: Cursor, gone: Gone, shape: Shape): Promise<void> {
	let more = true;
	while (more && !gone.current) {
		more = await pull(cursor, gone, shape);
	}
}

async function spin(cursor: Cursor, gone: Gone, shape: Shape): Promise<void> {
	while (!gone.current) {
		try {
			await drain(cursor, gone, shape);
		} catch {
			shape((now) => (gone.current ? now : now.failed()));
		}
		await pause(PACE);
	}
}

async function deliver(
	content: string,
	key: string,
	shape: Shape,
): Promise<void> {
	let answer: Response;
	try {
		answer = await fetch("/api/v1/window/im/send", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({ content, client_message_id: key }),
		});
	} catch {
		shape((now) => now.lost(key));
		return;
	}
	if (!answer.ok) {
		const body = await answer.json().catch(() => null);
		shape((now) => now.refused(key, answer.status, body?.code ?? null));
		return;
	}
	const note: Accepted | null = await answer.json().catch(() => null);
	shape((now) => (note === null ? now.lost(key) : now.accepted(key, note)));
}

async function roster(): Promise<Raw[]> {
	const answer = await fetch("/api/v1/strands", {
		headers: { Accept: "application/json" },
	});
	if (!answer.ok) {
		throw new Error(String(answer.status));
	}
	const strands: Raw[] = await answer.json();
	strands.sort((a, b) =>
		text(b, "updated_at").localeCompare(text(a, "updated_at")),
	);
	return strands;
}

async function replay(id: string, gone: Gone, mold: Mold): Promise<void> {
	const answer = await fetch(
		`/api/v1/strands/${encodeURIComponent(id)}/runtime`,
		{
			headers: { Accept: "application/json" },
		},
	);
	if (!answer.ok) {
		throw new Error(String(answer.status));
	}
	const snap = await answer.json();
	if (!gone.current) {
		mold((now) => now.replayed(snap));
	}
}

function subscribe(id: string, gone: Gone, mold: Mold): EventSource {
	const source = new EventSource(
		`/api/v1/strands/${encodeURIComponent(id)}/events`,
	);
	source.onopen = () => mold((now) => (gone.current ? now : now.opened()));
	source.onerror = () => mold((now) => (gone.current ? now : now.failed()));
	for (const name of NAMES) {
		source.addEventListener(name, (event) => {
			const env = JSON.parse((event as MessageEvent).data ?? "null");
			mold((now) => (gone.current ? now : now.handled(env)));
		});
	}
	return source;
}

export function Window() {
	const [face, shape] = useState<Chat>(Chat.opening());
	const [feed, mold] = useState<Feed>(Feed.opening());
	const [strands, stock] = useState<Raw[]>([]);
	const [active, aim] = useState<string | null>(null);
	const cursor = useRef(0);
	const gone = useRef(false);
	const wire = useRef<EventSource | null>(null);

	useEffect(() => {
		gone.current = false;
		spin(cursor, gone, shape);
		roster()
			.then(stock)
			.catch(() => stock([]));
		return () => {
			gone.current = true;
			wire.current?.close();
		};
	}, []);

	const pick = (strand: Raw) => {
		const id = text(strand, "id");
		aim(id);
		mold(() => Feed.opening());
		wire.current?.close();
		replay(id, gone, mold).catch(() => mold((now) => now.failed()));
		wire.current = subscribe(id, gone, mold);
	};

	const send = (content: string) => {
		const key = crypto.randomUUID();
		shape((now) => now.drafted(key, content));
		deliver(content, key, shape);
	};

	const again = (line: Line) => {
		if (face.busy) {
			return;
		}
		shape((now) => now.retried(line.key));
		deliver(line.text, line.key, shape);
	};

	return (
		<article>
			<h1>
				santi · window <Pill live={active !== null ? feed.live : face.live} />
			</h1>
			<Deck
				left={
					<div>
						<Roll strands={strands} active={active} pick={pick} />
						{active !== null && <Reel occs={feed.occs} turn={feed.turn} />}
						{active === null && <p>选择一个 strand 查看事件流。</p>}
					</div>
				}
				right={
					<div>
						{face.lines.map((line) => (
							<Bubble key={line.key} line={line} again={again} />
						))}
						{face.empty && <p>还没有对话。说点什么开始。</p>}
						<Composer frozen={face.frozen} busy={face.busy} send={send} />
					</div>
				}
			/>
		</article>
	);
}
