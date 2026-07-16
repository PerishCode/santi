import { readFileSync } from "node:fs";
import {
	createServer,
	type IncomingMessage,
	type Server,
	type ServerResponse,
} from "node:http";
import { join } from "node:path";
import type { Entry } from "../../src/lib/wire";

export interface Call {
	method: string;
	path: string;
	body: string;
	at: number;
}

export interface Answer {
	status: number;
	body: unknown;
	stall?: number;
	kill?: boolean;
}

export interface Plan {
	transcript: (count: number) => Answer;
	send?: (call: Call, count: number) => Answer;
}

export interface Rig {
	port: number;
	calls: Call[];
	peak: () => number;
	sends: () => Call[];
	polls: () => Call[];
	close: () => Promise<void>;
}

const DIST = join(import.meta.dirname, "../../dist");

function pause(ms: number): Promise<void> {
	return new Promise((settle) => setTimeout(settle, ms));
}

function slurp(feed: IncomingMessage): Promise<string> {
	return new Promise((settle) => {
		let body = "";
		feed.on("data", (piece) => {
			body += piece;
		});
		feed.on("end", () => settle(body));
	});
}

function file(
	reply: ServerResponse,
	name: string,
	kind: string,
	cache: string,
): boolean {
	try {
		const body = readFileSync(join(DIST, name));
		reply.writeHead(200, { "Content-Type": kind, "Cache-Control": cache });
		reply.end(body);
		return true;
	} catch {
		return false;
	}
}

function page(reply: ServerResponse): void {
	if (!file(reply, "index.html", "text/html; charset=utf-8", "no-cache")) {
		reply.writeHead(404);
		reply.end();
	}
}

function asset(reply: ServerResponse, rest: string): void {
	const kind = rest.endsWith(".css") ? "text/css" : "text/javascript";
	if (
		!file(
			reply,
			join("assets", rest),
			kind,
			"public, max-age=31536000, immutable",
		)
	) {
		reply.writeHead(404);
		reply.end();
	}
}

function json(reply: ServerResponse, status: number, body: unknown): void {
	reply.writeHead(status, { "Content-Type": "application/json" });
	reply.end(JSON.stringify(body));
}

export function stage(plan: Plan): Promise<Rig> {
	const calls: Call[] = [];
	let inflight = 0;
	let crest = 0;
	let pulls = 0;
	let posts = 0;

	const answer = async (
		feed: IncomingMessage,
		reply: ServerResponse,
		made: Answer,
	): Promise<void> => {
		if (made.stall !== undefined) {
			await pause(made.stall);
		}
		if (made.kill === true) {
			feed.socket.destroy();
			return;
		}
		json(reply, made.status, made.body);
	};

	const serve = async (
		feed: IncomingMessage,
		reply: ServerResponse,
	): Promise<void> => {
		reply.setHeader("Connection", "close");
		const path = feed.url ?? "/";
		const body = await slurp(feed);
		calls.push({ method: feed.method ?? "GET", path, body, at: Date.now() });
		if (path.startsWith("/api/v1/window/im/transcript")) {
			inflight += 1;
			crest = Math.max(crest, inflight);
			pulls += 1;
			const made = plan.transcript(pulls);
			await answer(feed, reply, made);
			inflight -= 1;
			return;
		}
		if (path === "/api/v1/window/im/send") {
			posts += 1;
			const made = plan.send
				? plan.send(calls[calls.length - 1], posts)
				: { status: 500, body: { code: "unplanned" } };
			await answer(feed, reply, made);
			return;
		}
		if (path === "/api/v1/strands") {
			json(reply, 200, []);
			return;
		}
		if (path.startsWith("/api/")) {
			json(reply, 404, { error: "unknown api path" });
			return;
		}
		if (path === "/") {
			page(reply);
			return;
		}
		if (path === "/panel") {
			reply.writeHead(308, { Location: "/panel/" });
			reply.end();
			return;
		}
		if (path === "/panel/") {
			page(reply);
			return;
		}
		if (path.startsWith("/assets/")) {
			asset(reply, path.slice("/assets/".length));
			return;
		}
		if (path.startsWith("/panel/assets/")) {
			asset(reply, path.slice("/panel/assets/".length));
			return;
		}
		reply.writeHead(404);
		reply.end();
	};

	const server: Server = createServer((feed, reply) => {
		serve(feed, reply);
	});

	return new Promise((settle) => {
		server.listen(0, "127.0.0.1", () => {
			const spot = server.address();
			const port = typeof spot === "object" && spot !== null ? spot.port : 0;
			settle({
				port,
				calls,
				peak: () => crest,
				sends: () =>
					calls.filter((call) => call.path === "/api/v1/window/im/send"),
				polls: () =>
					calls.filter((call) =>
						call.path.startsWith("/api/v1/window/im/transcript"),
					),
				close: () => new Promise((done) => server.close(() => done())),
			});
		});
	});
}

export function sheet(entries: Entry[], more = false, none = false): Answer {
	return {
		status: 200,
		body: {
			participant: "window:fixture",
			entries,
			next_since: entries.length > 0 ? entries[entries.length - 1].seq : 0,
			has_more: more,
			empty: none,
		},
	};
}
