import { afterEach, describe, expect, it } from "vitest";
import { RobledoError } from "../../src/index.js";
import {
	checkForUpdate,
	markReady,
	NATIVE_MODULE_NAME,
	type NativeRobledo,
	updateStatus,
	useNative,
} from "../../src/mobile/index.js";
import { modules } from "./reactNative.js";

class FakeNative implements NativeRobledo {
	current: string | null = null;
	pending: string | null = null;
	staged: Array<{
		manifest: string;
		signature: string;
		paths: string[];
		sizes: number[];
	}> = [];
	readies = 0;
	refuse: string | null = null;

	status(): string {
		return JSON.stringify({
			url: "https://api.example.com/__robledo",
			runtimeVersion: "rt1",
			platform: "android",
			current: this.current,
			pending: this.pending,
		});
	}
	stage(
		manifest: string,
		signature: string,
		paths: readonly string[],
		files: readonly ArrayBuffer[],
	): string {
		if (this.refuse) throw new Error(this.refuse);
		this.staged.push({
			manifest,
			signature,
			paths: [...paths],
			sizes: files.map((f) => f.byteLength),
		});
		this.pending = JSON.parse(manifest).id;
		return this.pending ?? "";
	}
	ready(): void {
		this.readies++;
	}
}

const manifest = JSON.stringify({
	id: "u2",
	files: [{ path: "index.android.bundle" }, { path: "assets/logo 1.png" }],
});

function server(answers: {
	[url: string]: { status: number; body?: unknown; bytes?: number };
}) {
	const asked: string[] = [];
	const fetch = async (url: string) => {
		asked.push(url);
		const answer = answers[url] ?? { status: 404 };
		return {
			status: answer.status,
			ok: answer.status >= 200 && answer.status < 300,
			json: async () => answer.body,
			arrayBuffer: async () => new ArrayBuffer(answer.bytes ?? 0),
		};
	};
	return { fetch, asked };
}

const base = "https://api.example.com/__robledo/android/rt1";

afterEach(() => useNative(null));

describe("checkForUpdate", () => {
	it("downloads the newest update's files and stages them for the next start", async () => {
		const native = new FakeNative();
		useNative(native);
		const { fetch, asked } = server({
			[`${base}/latest`]: { status: 200, body: { manifest, signature: "sig" } },
			[`${base}/u2/files/index.android.bundle`]: { status: 200, bytes: 10 },
			[`${base}/u2/files/assets/logo%201.png`]: { status: 200, bytes: 3 },
		});
		expect(await checkForUpdate({ fetch })).toEqual({
			kind: "staged",
			id: "u2",
		});
		expect(native.staged).toEqual([
			{
				manifest,
				signature: "sig",
				paths: ["index.android.bundle", "assets/logo 1.png"],
				sizes: [10, 3],
			},
		]);
		expect(asked).toHaveLength(3);
		// Asked again: already staged, nothing downloaded.
		expect(await checkForUpdate({ fetch })).toEqual({
			kind: "staged",
			id: "u2",
		});
		expect(asked).toHaveLength(4);
	});

	it("does nothing when there is no update or it is the one running", async () => {
		const native = new FakeNative();
		useNative(native);
		expect(
			await checkForUpdate({
				fetch: server({ [`${base}/latest`]: { status: 204 } }).fetch,
			}),
		).toEqual({ kind: "none" });
		native.current = "u2";
		expect(
			await checkForUpdate({
				fetch: server({
					[`${base}/latest`]: {
						status: 200,
						body: { manifest, signature: "s" },
					},
				}).fetch,
			}),
		).toEqual({ kind: "current", id: "u2" });
		expect(native.staged).toEqual([]);
	});

	it("names a bad answer, and what the phone refused", async () => {
		const native = new FakeNative();
		useNative(native);
		const code = async (run: () => Promise<unknown>) => {
			try {
				await run();
			} catch (error) {
				return error instanceof RobledoError ? error.code : String(error);
			}
			return undefined;
		};
		expect(
			await code(() =>
				checkForUpdate({
					fetch: server({ [`${base}/latest`]: { status: 500 } }).fetch,
				}),
			),
		).toBe("E_ROBLEDO_BAD_RESPONSE");
		expect(
			await code(() =>
				checkForUpdate({
					fetch: server({
						[`${base}/latest`]: { status: 200, body: { nope: 1 } },
					}).fetch,
				}),
			),
		).toBe("E_ROBLEDO_BAD_RESPONSE");
		expect(
			await code(() =>
				checkForUpdate({
					fetch: server({
						[`${base}/latest`]: {
							status: 200,
							body: { manifest, signature: "s" },
						},
					}).fetch,
				}),
			),
		).toBe("E_ROBLEDO_BAD_RESPONSE");
		native.refuse = "the manifest's signature does not match the app's key";
		const all = server({
			[`${base}/latest`]: { status: 200, body: { manifest, signature: "s" } },
			[`${base}/u2/files/index.android.bundle`]: { status: 200 },
			[`${base}/u2/files/assets/logo%201.png`]: { status: 200 },
		});
		expect(await code(() => checkForUpdate({ fetch: all.fetch }))).toBe(
			"E_ROBLEDO_REFUSED",
		);
	});
});

describe("the native module", () => {
	it("is found through React Native's registry, and its absence is named", () => {
		modules.delete(NATIVE_MODULE_NAME);
		expect(() => updateStatus()).toThrow(/TurboModuleRegistry/);
		modules.set(NATIVE_MODULE_NAME, { status: "not a function" });
		expect(() => updateStatus()).toThrow(RobledoError);
		const native = new FakeNative();
		modules.set(NATIVE_MODULE_NAME, native);
		expect(updateStatus().runtimeVersion).toBe("rt1");
		markReady();
		expect(native.readies).toBe(1);
	});
});
