import type { Robledo } from "./Robledo.js";

/** The part of a host's HTTP context robledo's routes use (ream's, structurally). */
export interface RobledoHttpContext {
	request: {
		param(key: string): string | undefined;
		params(): Readonly<{ [key: string]: string | string[] | undefined }>;
	};
	response: {
		status(code: number): unknown;
		header(key: string, value: string): unknown;
		json(data: unknown): void;
		noContent(): void;
		send(body: string): void;
		download(path: string): void;
	};
}

export interface RobledoRouter {
	get(
		path: string,
		handler: (ctx: RobledoHttpContext) => Promise<void> | void,
	): unknown;
}

function wildcard(ctx: RobledoHttpContext): string[] {
	const value = ctx.request.params()["*"];
	if (Array.isArray(value)) return value;
	return typeof value === "string" ? value.split("/") : [];
}

/** Mount robledo's routes on `router` (see `Robledo.registerRoutes`). */
export function registerRobledoRoutes(
	router: RobledoRouter,
	robledo: Robledo,
): void {
	router.get(`${robledo.prefix}/:platform/:runtime/latest`, async (ctx) => {
		const latest = await robledo.latest(
			ctx.request.param("platform") ?? "",
			ctx.request.param("runtime") ?? "",
		);
		// Asked at every start: never cached, or a phone keeps an old answer.
		ctx.response.header("cache-control", "no-store");
		if (latest === null) {
			ctx.response.noContent();
			return;
		}
		ctx.response.json(latest);
	});

	router.get(`${robledo.prefix}/:platform/:runtime/:id/files/*`, (ctx) => {
		const path = robledo.filePath(
			ctx.request.param("platform") ?? "",
			ctx.request.param("runtime") ?? "",
			ctx.request.param("id") ?? "",
			wildcard(ctx),
		);
		if (path === null) {
			ctx.response.status(404);
			ctx.response.send("Not Found");
			return;
		}
		// An update's files never change under its id.
		ctx.response.header("cache-control", "public, max-age=31536000, immutable");
		ctx.response.header("content-type", "application/octet-stream");
		ctx.response.download(path);
	});
}
