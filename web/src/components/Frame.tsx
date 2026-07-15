import type { ReactNode } from "react";
import "./tokens.scss";
import "./themes/glass.scss";
import "./media.scss";
import "./Frame.scss";

type Props = {
	children: ReactNode;
};

export function Frame(props: Props) {
	return <div className="frame">{props.children}</div>;
}
