import type { Line } from "../../lib/chat";
import "./Bubble.scss";

type Props = {
	line: Line;
	again: (line: Line) => void;
};

function badge(line: Line): string | null {
	if (line.state === null) {
		return null;
	}
	if (line.state === "settled") {
		return "accepted · 已入时间线";
	}
	if (line.state === "accepted") {
		return line.at ? `accepted · ${clock(line.at)}` : "accepted";
	}
	if (line.state === "submitting") {
		return "submitting…";
	}
	return `${line.state}${line.reason ? ` — ${line.reason}` : ""}`;
}

function clock(at: string): string {
	const when = new Date(at);
	return Number.isNaN(when.getTime()) ? at : when.toLocaleTimeString();
}

export function Bubble(props: Props) {
	const line = props.line;
	const tone =
		line.state === "unknown" || line.state === "rejected"
			? ` ${line.state}`
			: "";
	const note = badge(line);
	return (
		<div className={`bubble ${line.author}${tone}`}>
			<div className="who">
				{line.author === "human" ? "you" : "liberte"}
				{line.at !== null && line.state === null && (
					<span className="when">{clock(line.at)}</span>
				)}
			</div>
			<pre>{line.text}</pre>
			{note !== null && (
				<div className="note">
					{note}
					{line.retry && (
						<button type="button" onClick={() => props.again(line)}>
							重试
						</button>
					)}
				</div>
			)}
		</div>
	);
}
