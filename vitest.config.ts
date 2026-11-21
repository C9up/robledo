import { fileURLToPath } from "node:url";
import { defineConfig } from "vitest/config";

export default defineConfig({
	resolve: {
		// The mobile entry imports React Native, which only an app installs.
		alias: {
			"react-native": fileURLToPath(
				new URL("./tests/unit/reactNative.ts", import.meta.url),
			),
		},
	},
	test: {
		coverage: {
			provider: "v8",
			include: ["src/**"],
			exclude: ["src/**/*.d.ts"],
			reporter: ["text-summary", "json-summary"],
			thresholds: { lines: 85, statements: 85, branches: 80, functions: 85 },
		},
	},
});
