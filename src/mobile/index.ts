/**
 * `@c9up/robledo/mobile` — the app's side of over-the-air updates.
 *
 *     import { checkForUpdate, markReady } from "@c9up/robledo/mobile";
 *
 *     // once the first screen is up (the root component's onMount):
 *     markReady();
 *     void checkForUpdate();
 *
 * `checkForUpdate` asks the app's ream server for the newest update built for
 * this app's native runtime, downloads its files and hands them to the
 * native side, which verifies the signature and every digest before keeping
 * it. It takes effect at the next start. `markReady` tells robledo the
 * current update reached its first screen: an update that never does is
 * abandoned at the start after, and the previous one runs again.
 */
import { TurboModuleRegistry } from "react-native";
import {
	E_ROBLEDO_BAD_RESPONSE,
	E_ROBLEDO_NO_NATIVE_MODULE,
	E_ROBLEDO_REFUSED,
} from "../errors.js";

export * as errors from "../errors.js";

export const NATIVE_MODULE_NAME = "NativeRobledo";

/** The native module, structurally (see specs/NativeRobledo.ts). */
export interface NativeRobledo {
	status(): string;
	stage(
		manifest: string,
		signature: string,
		paths: readonly string[],
		files: readonly ArrayBuffer[],
	): string;
	ready(): void;
}

export interface UpdateStatus {
	url: string;
	runtimeVersion: string;
	platform: string;
	/** The update running now, or null for the bundle inside the app. */
	current: string | null;
	/** An update staged for the next start. */
	pending: string | null;
}

export type CheckResult =
	| { kind: "none" }
	| { kind: "current"; id: string }
	| { kind: "staged"; id: string };

type Fetch = (url: string) => Promise<{
	status: number;
	ok: boolean;
	json(): Promise<unknown>;
	arrayBuffer(): Promise<ArrayBuffer>;
}>;

let override: NativeRobledo | null = null;

/** @internal Tests give their own native module. */
export function useNative(native: NativeRobledo | null): void {
	override = native;
}

function isNative(value: unknown): value is NativeRobledo {
	return (
		typeof value === "object" &&
		value !== null &&
		["status", "stage", "ready"].every(
			(name) => typeof Reflect.get(value, name) === "function",
		)
	);
}

function native(): NativeRobledo {
	if (override) return override;
	const module = TurboModuleRegistry.getEnforcing(NATIVE_MODULE_NAME);
	if (!isNative(module)) {
		throw new E_ROBLEDO_NO_NATIVE_MODULE();
	}
	return module;
}

const text = (value: unknown, key: string): string => {
	const v: unknown =
		typeof value === "object" && value !== null
			? Reflect.get(value, key)
			: undefined;
	return typeof v === "string" ? v : "";
};

export function updateStatus(): UpdateStatus {
	const parsed: unknown = JSON.parse(native().status());
	const id = (key: string) => text(parsed, key) || null;
	return {
		url: text(parsed, "url"),
		runtimeVersion: text(parsed, "runtimeVersion"),
		platform: text(parsed, "platform"),
		current: id("current"),
		pending: id("pending"),
	};
}

/** The files a manifest lists, read without trusting it: the native side checks it. */
function listedFiles(manifest: string): { id: string; paths: string[] } {
	const parsed: unknown = JSON.parse(manifest);
	const files: unknown =
		typeof parsed === "object" && parsed !== null
			? Reflect.get(parsed, "files")
			: undefined;
	const paths = Array.isArray(files)
		? files.map((file) => text(file, "path")).filter((p) => p !== "")
		: [];
	return { id: text(parsed, "id"), paths };
}

/** Ask the server for a newer update and stage it for the next start. */
export async function checkForUpdate(
	options: { fetch?: Fetch } = {},
): Promise<CheckResult> {
	const get: Fetch = options.fetch ?? globalThis.fetch;
	const status = updateStatus();
	const base = `${status.url}/${status.platform}/${encodeURIComponent(status.runtimeVersion)}`;
	const response = await get(`${base}/latest`);
	if (response.status === 204) return { kind: "none" };
	if (!response.ok) {
		throw new E_ROBLEDO_BAD_RESPONSE([
			`${base}/latest answered ${response.status}`,
		]);
	}
	const body: unknown = await response.json();
	const manifest = text(body, "manifest");
	const signature = text(body, "signature");
	if (manifest === "" || signature === "") {
		throw new E_ROBLEDO_BAD_RESPONSE([
			`${base}/latest did not answer a manifest and a signature`,
		]);
	}
	const { id, paths } = listedFiles(manifest);
	if (id === status.current) return { kind: "current", id };
	if (id === status.pending) return { kind: "staged", id };
	const files = await Promise.all(
		paths.map(async (path) => {
			const url = `${base}/${encodeURIComponent(id)}/files/${path.split("/").map(encodeURIComponent).join("/")}`;
			const file = await get(url);
			if (!file.ok) {
				throw new E_ROBLEDO_BAD_RESPONSE([`${url} answered ${file.status}`]);
			}
			return file.arrayBuffer();
		}),
	);
	try {
		return {
			kind: "staged",
			id: native().stage(manifest, signature, paths, files),
		};
	} catch (error) {
		throw new E_ROBLEDO_REFUSED(
			[id, error instanceof Error ? error.message : String(error)],
			{ cause: error },
		);
	}
}

/** The current update reached its first screen: keep it. */
export function markReady(): void {
	native().ready();
}
