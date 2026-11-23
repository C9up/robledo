import { describe, expect, it } from "vitest";
import { errors, Robledo } from "../../src/index.js";
import RobledoProvider from "../../src/RobledoProvider.js";
import robledo from "../../src/services/main.js";

class FakeContainer {
	readonly factories = new Map<unknown, () => unknown>();
	readonly instances = new Map<unknown, unknown>();
	singleton(token: unknown, factory: () => unknown): void {
		this.factories.set(token, factory);
	}
	async resolve<T = unknown>(token: unknown): Promise<T> {
		if (!this.instances.has(token))
			this.instances.set(token, await this.factories.get(token)?.());
		// The container returns what was bound; tests bind what they read.
		const value: unknown = this.instances.get(token);
		return value as T;
	}
	has(token: unknown): boolean {
		return this.factories.has(token) || this.instances.has(token);
	}
}

function app(
	config: { [key: string]: unknown },
	router?: { get: (path: string) => void },
) {
	const container = new FakeContainer();
	if (router) container.instances.set("router", router);
	return {
		container,
		config: { get: <T>(key: string) => config[key] as T | undefined },
		makePath: (...segments: string[]) => ["/srv/app", ...segments].join("/"),
	};
}

describe("RobledoProvider", () => {
	it("binds the instance from config, resolved against the app's root, and serves it from services/main", async () => {
		const host = app({
			robledo: { directory: "storage/updates", prefix: "/updates" },
		});
		const provider = new RobledoProvider(host);
		provider.register();
		await provider.boot();
		const instance = await host.container.resolve<Robledo>(Robledo);
		expect(instance.directory).toBe("/srv/app/storage/updates");
		expect(instance.prefix).toBe("/updates");
		expect(await host.container.resolve("robledo")).toBe(instance);
		expect(robledo.prefix).toBe("/updates");
		await provider.shutdown();
		expect(() => robledo.prefix).toThrow(errors.E_ROBLEDO_NOT_BOOTED);
	});

	it("mounts the routes on the host router once the application asks", async () => {
		const paths: string[] = [];
		const host = app({}, { get: (path) => paths.push(path) });
		const provider = new RobledoProvider(host);
		provider.register();
		await provider.boot();
		expect(paths).toEqual([]);
		robledo.registerRoutes();
		expect(robledo.hasRegisteredRoutes()).toBe(true);
		expect(paths).toEqual([
			"/__robledo/:platform/:runtime/latest",
			"/__robledo/:platform/:runtime/:id/files/*",
		]);
		robledo.registerRoutes();
		expect(paths).toHaveLength(2);
		await provider.shutdown();
	});

	it("reads as no thenable before it is booted", () => {
		expect(Reflect.get(robledo, "then")).toBeUndefined();
		expect(Reflect.get(robledo, Symbol.toStringTag)).toBeUndefined();
	});
});
