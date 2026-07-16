import type { ReactNode } from "react";
import "./Deck.scss";

type Props = {
	left: ReactNode;
	right: ReactNode;
};

export function Deck(props: Props) {
	return (
		<div className="deck">
			<div className="port">{props.left}</div>
			<div className="star">{props.right}</div>
		</div>
	);
}
