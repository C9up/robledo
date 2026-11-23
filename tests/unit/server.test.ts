import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import {
	errors,
	Robledo,
	type RobledoHttpContext,
	registerRobledoRoutes,
} from "../../src/index.js";

function published(): string {
	const root = mkdtempSync(join(tmpdir(), "robledo-"));
	const dir = join(root, "robledo", "android", "rt1");
	mkdirSync(join(dir, "u2", "files", "assets"), { recursive: true });
	writeFileSync(join(dir, "latest"), "u2\n");
	writeFileSync(join(dir, "u2", "manifest.json"), '{"id":"u2"}');
	writeFileSync(join(dir, "u2", "manifest.sig"), "c2lnbmF0dXJl\n");
	writeFileSync(join(dir, "u2", "files", "index.android.bundle"), "code");
	writeFileSync(join(dir, "u2", "files", "assets", "logo.png"), "png");
	return root;
}

function robledoAt(root: string): Robledo {
	return new Robledo({}, (path) => join(root, path));
}

describe("Robledo on the server", () => {
	it("answers the newest update for a platform and runtime, as published", async () => {
		const robledo = robledoAt(published());
		expect(await robledo.latest("android", "rt1")).toEqual({
			manifest: '{"id":"u2"}',
			signature: "c2lnbmF0dXJl",
		});
		expect(await robledo.latest("android", "rt9")).toBeNull();
		expect(await robledo.latest("ios", "rt1")).toBeNull();
		expect(await robledo.latest("windows", "rt1")).toBeNull();
		expect(await robledo.latest("android", "../rt1")).toBeNull();
	});

	it("resolves a file inside its update, and nothing outside it", () => {
		const root = published();
		const robledo = robledoAt(root);
		expect(
			robledo.filePath("android", "rt1", "u2", ["assets", "logo.png"]),
		).toBe(
			join(
				root,
				"robledo",
				"android",
				"rt1",
				"u2",
				"files",
				"assets",
				"logo.png",
			),
		);
		for (const segments of [
			["..", "manifest.sig"],
			["."],
			["a\\b"],
			[""],
			[],
			["a/b"],
		]) {
			expect(robledo.filePath("android", "rt1", "u2", segments)).toBeNull();
		}
		expect(robledo.filePath("android", "rt1", "../u2", ["x"])).toBeNull();
		expect(robledo.filePath("android", "rt.1", "u2", ["x"])).toBeNull();
	});

	it("refuses a prefix that is not a path", () => {
		expect(() => new Robledo({ prefix: "robledo" })).toThrow(
			errors.E_ROBLEDO_INVALID_PREFIX,
		);
		expect(() => new Robledo({ prefix: "/robledo/" })).toThrow(
			errors.E_ROBLEDO_INVALID_PREFIX,
		);
	});

	it("mounts its routes when the application asks, before or after the router came", async () => {
		const robledo = robledoAt(published());
		const routes = new Map<string, (ctx: RobledoHttpContext) => unknown>();
		robledo.registerRoutes();
		robledo.useMounter((target) =>
			registerRobledoRoutes(
				{ get: (path, handler) => routes.set(path, handler) },
				target,
			),
		);
		expect([...routes.keys()]).toEqual([
			"/__robledo/:platform/:runtime/latest",
			"/__robledo/:platform/:runtime/:id/files/*",
		]);

		const answer: { [key: string]: unknown } = { headers: {} };
		const headers: { [key: string]: string } = {};
		const ctx = (params: {
			[key: string]: string | string[];
		}): RobledoHttpContext => ({
			request: {
				param: (key) => {
					const v = params[key];
					return typeof v === "string" ? v : undefined;
				},
				params: () => params,
			},
			response: {
				status: (code) => {
					answer.status = code;
				},
				header: (key, value) => {
					headers[key] = value;
				},
				json: (data) => {
					answer.json = data;
				},
				noContent: () => {
					answer.status = 204;
				},
				send: (body) => {
					answer.body = body;
				},
				download: (path) => {
					answer.download = path;
				},
			},
		});
		await routes.get("/__robledo/:platform/:runtime/latest")?.(
			ctx({ platform: "android", runtime: "rt1" }),
		);
		expect(answer.json).toEqual({
			manifest: '{"id":"u2"}',
			signature: "c2lnbmF0dXJl",
		});
		expect(headers["cache-control"]).toBe("no-store");
		await routes.get("/__robledo/:platform/:runtime/latest")?.(
			ctx({ platform: "android", runtime: "none" }),
		);
		expect(answer.status).toBe(204);

		await routes.get("/__robledo/:platform/:runtime/:id/files/*")?.(
			ctx({
				platform: "android",
				runtime: "rt1",
				id: "u2",
				"*": ["index.android.bundle"],
			}),
		);
		expect(String(answer.download)).toMatch(
			/u2\/files\/index\.android\.bundle$/,
		);
		expect(headers["cache-control"]).toContain("immutable");
		await routes.get("/__robledo/:platform/:runtime/:id/files/*")?.(
			ctx({
				platform: "android",
				runtime: "rt1",
				id: "u2",
				"*": ["..", "manifest.sig"],
			}),
		);
		expect(answer.status).toBe(404);
	});
});
