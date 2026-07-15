export type Author = "human" | "assistant";

export type Send = {
	content: string;
	client_message_id: string;
};

export type Accepted = {
	status: string;
	message_id: string;
	client_message_id: string;
	receipt_id: string;
	received_at: string;
	cursor?: number | null;
};

export type Transcript = {
	participant: string;
	entries: Entry[];
	next_since: number;
	has_more: boolean;
	empty: boolean;
};

export type Entry = {
	message_id: string;
	seq: number;
	author: Author;
	text: string;
	at: string;
};
