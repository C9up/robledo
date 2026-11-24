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
 * this app's native runtime and has the native side check its signed
 * manifest — signature, runtime, and that it is newer than anything this
 * phone accepted — before it fetches any file. The files then come a few at a
 * time, each no larger than the manifest says, and go to the native side as
 * they arrive, which checks every digest before keeping the update. It takes
 * effect at the next start. `markReady` tells robledo the current update
 * reached its first screen: an update that never does is abandoned at the
 * start after, and the previous one runs again.
 */
import { TurboModuleRegistry } from "react-native";
import {
	E_ROBLEDO_BAD_RESPONSE,
	E_ROBLEDO_NO_NATIVE_MODULE,
	E_ROBLEDO_NOT_NEWER,
	E_ROBLEDO_REFUSED,
} from "../errors.js";

export * as errors from "../errors.js";

export const NATIVE_MODULE_NAME = "NativeRobledo";

/** The native module, structurally (see specs/NativeRobledo.ts). */
export interface NativeRobledo {
	status(): string;
	/** Check a manifest before any file is fetched: `{"id", "staging"}` as JSON, or throws why not. */
	verify(manifest: string, signature: string): string;
	/** One file of the update `verify` accepted, checked against its signed size and digest. */
	receive(path: string, file: ArrayBuffer): void;
	/** Every file arrived: keep the update for the next start; returns its id. */
	finish(): string;
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

/** What `checkForUpdate` reads of a response: `fetch`'s, React Native's included. */
interface FetchResponse {
	status: number;
	ok: boolean;
	headers?: { get(name: string): string | null };
	/** Read chunk by chunk when the platform streams (React Native does not). */
	body?: {
		getReader(): {
			read(): Promise<{ done: boolean; value?: Uint8Array }>;
			cancel(): Promise<void>;
		};
	} | null;
	json(): Promise<unknown>;
	arrayBuffer(): Promise<ArrayBuffer>;
}

type Fetch = (url: string) => Promise<FetchResponse>;

/** The largest `latest` answer read, and how many files are fetched at once. */
export const MAX_LATEST_BYTES = 1024 * 1024;
export const PARALLEL_DOWNLOADS = 4;

let override: NativeRobledo | null = null;

/** @internal Tests give their own native module. */
export function useNative(native: NativeRobledo | null): void {
	override = native;
}

function isNative(value: unknown): value is NativeRobledo {
	return (
		typeof value === "object" &&
		value !== null &&
		["status", "verify", "receive", "finish", "ready"].every(
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

/** The files of a manifest the native side verified: path and signed size. */
function listedFiles(manifest: string): Array<{ path: string; size: number }> {
	const parsed: unknown = JSON.parse(manifest);
	const files: unknown =
		typeof parsed === "object" && parsed !== null
			? Reflect.get(parsed, "files")
			: undefined;
	if (!Array.isArray(files)) return [];
	return files.map((file: unknown) => {
		const size: unknown =
			typeof file === "object" && file !== null
				? Reflect.get(file, "size")
				: undefined;
		return {
			path: text(file, "path"),
			size: typeof size === "number" ? size : 0,
		};
	});
}

/** A length a response announces, or null. */
function announced(response: FetchResponse): number | null {
	const value = response.headers?.get("content-length");
	if (value === null || value === undefined || value.trim() === "") return null;
	const length = Number(value);
	return Number.isFinite(length) ? length : null;
}

/**
 * A response's body, refused as soon as it runs past `limit` bytes: at once
 * when it announces more, chunk by chunk where the platform streams, and in
 * any case once read.
 */
async function bounded(
	response: FetchResponse,
	limit: number,
	url: string,
): Promise<ArrayBuffer> {
	const tooLarge = () =>
		new E_ROBLEDO_BAD_RESPONSE([
			`${url} is larger than the ${limit} bytes expected`,
		]);
	const length = announced(response);
	if (length !== null && length > limit) throw tooLarge();
	const reader = response.body?.getReader();
	if (!reader) {
		const whole = await response.arrayBuffer();
		if (whole.byteLength > limit) throw tooLarge();
		return whole;
	}
	const out = new Uint8Array(limit);
	let size = 0;
	for (;;) {
		const { done, value } = await reader.read();
		if (done) break;
		if (!value) continue;
		if (size + value.byteLength > limit) {
			await reader.cancel();
			throw tooLarge();
		}
		out.set(value, size);
		size += value.byteLength;
	}
	return out.buffer.slice(0, size);
}

/** The id a manifest claims, for a refusal's message only (it was not verified). */
function claimedId(manifest: string): string {
	try {
		return text(JSON.parse(manifest), "id").slice(0, 64);
	} catch {
		return "?";
	}
}

/** Why the native side refused an update, as robledo's error for it. */
function refused(id: string, error: unknown): Error {
	const message = error instanceof Error ? error.message : String(error);
	const prefix = "E_ROBLEDO_NOT_NEWER: ";
	if (message.startsWith(prefix)) {
		return new E_ROBLEDO_NOT_NEWER([message.slice(prefix.length)], {
			cause: error,
		});
	}
	return new E_ROBLEDO_REFUSED([id, message], { cause: error });
}

/**
 * Ask the server for a newer update and stage it for the next start. Nothing
 * the server names is fetched before the native side verified the manifest's
 * signature; files then come `PARALLEL_DOWNLOADS` at a time, none larger than
 * its signed size, each handed to the native side as it arrives.
 */
export async function checkForUpdate(
	options: { fetch?: Fetch } = {},
): Promise<CheckResult> {
	const get: Fetch = options.fetch ?? globalThis.fetch;
	const status = updateStatus();
	const base = `${status.url}/${status.platform}/${encodeURIComponent(status.runtimeVersion)}`;
	const latest = `${base}/latest`;
	const response = await get(latest);
	if (response.status === 204) return { kind: "none" };
	if (!response.ok) {
		throw new E_ROBLEDO_BAD_RESPONSE([`${latest} answered ${response.status}`]);
	}
	const length = announced(response);
	if (length !== null && length > MAX_LATEST_BYTES) {
		throw new E_ROBLEDO_BAD_RESPONSE([
			`${latest} is larger than the ${MAX_LATEST_BYTES} bytes expected`,
		]);
	}
	const body: unknown = await response.json();
	const manifest = text(body, "manifest");
	const signature = text(body, "signature");
	if (manifest === "" || signature === "") {
		throw new E_ROBLEDO_BAD_RESPONSE([
			`${latest} did not answer a manifest and a signature`,
		]);
	}
	if (manifest.length > MAX_LATEST_BYTES) {
		throw new E_ROBLEDO_BAD_RESPONSE([
			`${latest} is larger than the ${MAX_LATEST_BYTES} bytes expected`,
		]);
	}
	let verified: unknown;
	try {
		verified = JSON.parse(native().verify(manifest, signature));
	} catch (error) {
		throw refused(claimedId(manifest), error);
	}
	const id = text(verified, "id");
	if (Reflect.get(Object(verified), "staging") !== true) {
		return { kind: id === status.current ? "current" : "staged", id };
	}
	// The manifest is the publisher's from here on: its paths and sizes hold.
	const queue = listedFiles(manifest);
	const fetchOne = async (path: string, size: number) => {
		const url = `${base}/${encodeURIComponent(id)}/files/${path.split("/").map(encodeURIComponent).join("/")}`;
		const file = await get(url);
		if (!file.ok) {
			throw new E_ROBLEDO_BAD_RESPONSE([`${url} answered ${file.status}`]);
		}
		const data = await bounded(file, size, url);
		try {
			native().receive(path, data);
		} catch (error) {
			throw refused(id, error);
		}
	};
	// One failure stops the others: nothing more is fetched for a refused update.
	let failed = false;
	const worker = async () => {
		for (let next = queue.shift(); next && !failed; next = queue.shift()) {
			try {
				await fetchOne(next.path, next.size);
			} catch (error) {
				failed = true;
				throw error;
			}
		}
	};
	await Promise.all(Array.from({ length: PARALLEL_DOWNLOADS }, worker));
	try {
		return { kind: "staged", id: native().finish() };
	} catch (error) {
		throw refused(id, error);
	}
}

/** The current update reached its first screen: keep it. */
export function markReady(): void {
	native().ready();
}
