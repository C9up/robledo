/** Stands in for React Native in tests. */
export const modules = new Map<string, unknown>();

export const TurboModuleRegistry = {
	getEnforcing(name: string): unknown {
		if (!modules.has(name))
			throw new Error(`TurboModuleRegistry: ${name} not found`);
		return modules.get(name);
	},
};
