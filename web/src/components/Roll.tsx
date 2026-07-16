import type { Raw } from "../lib/feed";
import { text } from "../lib/feed";
import "./Roll.scss";

type Props = {
	strands: Raw[];
	active: string | null;
	pick: (strand: Raw) => void;
};

function meta(strand: Raw): string {
	const label = text(strand, "external_label");
	const soul = text(strand, "soul_id").slice(0, 12);
	return `${label ? `${label} · ` : ""}soul ${soul}`;
}

export function Roll(props: Props) {
	return (
		<ul className="roll">
			{props.strands.map((strand) => {
				const id = text(strand, "id");
				return (
					<li key={id} className={id === props.active ? "active" : ""}>
						<button type="button" onClick={() => props.pick(strand)}>
							<span className="sid">{id}</span>
							<span className="smeta">{meta(strand)}</span>
						</button>
					</li>
				);
			})}
		</ul>
	);
}
