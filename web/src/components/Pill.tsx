import "./Pill.scss";

type Props = {
	live: boolean;
};

export function Pill(props: Props) {
	return (
		<span className={props.live ? "pill live" : "pill"}>
			{props.live ? "live" : "reconnecting…"}
		</span>
	);
}
