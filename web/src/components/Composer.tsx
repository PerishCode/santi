import { useState } from "react";
import "./Composer.scss";

type Props = {
	frozen: boolean;
	busy: boolean;
	send: (text: string) => void;
};

export function Composer(props: Props) {
	const [text, update] = useState("");
	const ready = !props.frozen && !props.busy && text.trim().length > 0;
	const fire = () => {
		if (!ready) {
			return;
		}
		props.send(text);
		update("");
	};
	return (
		<form
			className="composer"
			onSubmit={(event) => {
				event.preventDefault();
				fire();
			}}
		>
			<textarea
				value={text}
				disabled={props.frozen}
				placeholder={props.frozen ? "只读:此身份未被授予发言权限" : "说点什么…"}
				onChange={(event) => update(event.target.value)}
				onKeyDown={(event) => {
					if (event.key === "Enter" && !event.shiftKey) {
						event.preventDefault();
						fire();
					}
				}}
			/>
			<button type="submit" disabled={!ready}>
				发送
			</button>
		</form>
	);
}
