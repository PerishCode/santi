import { defineConfig } from "@playwright/test";

export default defineConfig({
	testDir: "../pane",
	timeout: 30000,
	fullyParallel: true,
	reporter: [["list"]],
	outputDir: "../../.local/playwright",
	use: {
		trace: "retain-on-failure",
	},
});
