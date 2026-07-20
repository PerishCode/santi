import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { Frame } from "./components/Frame/Frame";
import { resolve } from "./lib/route";

const View = resolve(window.location.hash);
const root = document.getElementById("root");
if (root !== null) {
	createRoot(root).render(
		<StrictMode>
			<Frame>
				<View />
			</Frame>
		</StrictMode>,
	);
}
