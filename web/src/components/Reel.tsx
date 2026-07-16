import { useEffect, useRef } from "react";
import type { Occ } from "../lib/feed";
import "./Reel.scss";

type Props = {
	occs: Occ[];
	turn: string | null;
};

function clock(at: string | null): string {
	if (at === null) {
		return "";
	}
	const when = new Date(at);
	return Number.isNaN(when.getTime()) ? at : when.toLocaleTimeString();
}

export function Reel(props: Props) {
	const box = useRef<HTMLDivElement>(null);
	const stick = useRef(true);

	useEffect(() => {
		const node = box.current;
		if (node !== null && stick.current) {
			node.scrollTop = node.scrollHeight;
		}
	});

	const drift = () => {
		const node = box.current;
		if (node !== null) {
			stick.current =
				node.scrollHeight - node.scrollTop - node.clientHeight < 48;
		}
	};

	return (
		<div className="reel" ref={box} onScroll={drift}>
			{props.occs.map((occ) => (
				<div key={occ.key} className={`occ ${occ.kind}`}>
					<div className="k">
						{occ.label}
						{occ.at !== null && <span className="t">{clock(occ.at)}</span>}
					</div>
					{occ.body !== "" && <pre>{occ.body}</pre>}
				</div>
			))}
			{props.turn !== null && <div className="occ turning">{props.turn}</div>}
		</div>
	);
}
