/**
 * The part of React Native's JS API robledo's mobile entry uses — declared here so
 * robledo builds without React Native installed. An app has the real package;
 * this file is not part of what robledo's declarations reference.
 */
declare module "react-native" {
	export const AppRegistry: {
		registerRunnable(
			name: string,
			run: (params: { initialProps?: { [key: string]: unknown } }) => void,
		): void;
	};
	export const Dimensions: {
		get(dimension: "window"): { width: number; height: number };
	};
	export const TurboModuleRegistry: {
		getEnforcing(name: string): unknown;
	};
}
