import { Robledo, type RobledoConfig } from "./server/Robledo.js";
import { type RobledoRouter, registerRobledoRoutes } from "./server/routes.js";
import { clearRobledo, getRobledo, setRobledo } from "./services/main.js";

interface RobledoContainer {
	singleton(token: unknown, factory: () => unknown): void;
	resolve<T = unknown>(token: unknown): Promise<T>;
	has(token: unknown): boolean;
}

export interface RobledoAppContext {
	container: RobledoContainer;
	config: { get<T = unknown>(key: string): T | undefined };
	/** Resolve against the application root (ream's `app.makePath`). */
	makePath?(...segments: string[]): string;
}

/**
 * Registers the `Robledo` singleton (config `robledo`: `directory`, `prefix`)
 * and gives it the host router. It mounts no route itself: the application
 * calls `robledo.registerRoutes()` from a preload, so the routes are there
 * before the server listens — as relay's are.
 *
 *     // reamrc.ts
 *     providers: [() => import("@c9up/robledo/provider")]
 *
 *     // start/routes.ts
 *     import robledo from "@c9up/robledo/services/main"
 *     robledo.registerRoutes()
 */
export default class RobledoProvider {
	#instance: Robledo | undefined;

	constructor(protected app: RobledoAppContext) {}

	register(): void {
		this.app.container.singleton(Robledo, () => {
			const config = this.app.config.get<RobledoConfig>("robledo") ?? {};
			const resolvePath = this.app.makePath?.bind(this.app);
			const instance = new Robledo(config, resolvePath);
			this.#instance = instance;
			setRobledo(instance);
			return instance;
		});
		this.app.container.singleton("robledo", () =>
			this.app.container.resolve<Robledo>(Robledo),
		);
	}

	async boot(): Promise<void> {
		const robledo = await this.app.container.resolve<Robledo>(Robledo);
		setRobledo(robledo);
		// A host without a router gets the instance and no routes.
		if (!this.app.container.has("router")) return;
		const router = await this.app.container.resolve<RobledoRouter>("router");
		robledo.useMounter((target) => registerRobledoRoutes(router, target));
	}

	async shutdown(): Promise<void> {
		if (this.#instance !== undefined && getRobledo() === this.#instance)
			clearRobledo();
	}
}
