import { Exception } from "@c9up/flare";
import { afterEach, describe, expect, it } from "vitest";
import {
	checkForUpdate,
	errors,
	MAX_LATEST_BYTES,
	markReady,
	NATIVE_MODULE_NAME,
	type NativeRobledo,
	PARALLEL_DOWNLOADS,
	updateStatus,
	useNative,
} from "../../src/mobile/index.js";
import { modules } from "./reactNative.js";

/**
 * Stands in for the native side: `verify` refuses what `refuse` says (the
 * signature check, the order…), records when it was asked, and takes the
 * files one by one as the Rust client does.
 */
class FakeNative implements NativeRobledo {
	current: string | null = null;
	pending: string | null = null;
	/** What happened, in order: "verify", "receive <path> <bytes>", "finish". */
	log: string[] = [];
	readies = 0;
	refuse: string | null = null;
	refuseFile: string | null = null;
	#receiving: string | null = null;

	status(): string {
		return JSON.stringify({
			url: "https://api.example.com/__robledo",
			runtimeVersion: "rt1",
			platform: "android",
			current: this.current,
			pending: this.pending,
		});
	}
	verify(manifest: string, _signature: string): string {
		this.log.push("verify");
		if (this.refuse) throw new Error(this.refuse);
		const id: unknown = JSON.parse(manifest).id;
		const known = id === this.current || id === this.pending;
		this.#receiving = known || typeof id !== "string" ? null : id;
		return JSON.stringify({ id, staging: !known });
	}
	receive(path: string, file: ArrayBuffer): void {
		if (this.refuseFile) throw new Error(this.refuseFile);
		this.log.push(`receive ${path} ${file.byteLength}`);
	}
	finish(): string {
		this.log.push("finish");
		this.pending = this.#receiving;
		return this.pending ?? "";
	}
	ready(): void {
		this.readies++;
	}
}

const manifest = JSON.stringify({
	id: "u2",
	createdAt: 1_780_272_000,
	files: [
		{ path: "index.android.bundle", size: 10 },
		{ path: "assets/logo 1.png", size: 3 },
	],
});

type Answer = {
	status: number;
	body?: unknown;
	bytes?: number;
	length?: string;
	chunks?: number[];
};

function server(answers: { [url: string]: Answer }) {
	const asked: string[] = [];
	let open = 0;
	let peak = 0;
	let cancelled = 0;
	const fetch = async (url: string) => {
		asked.push(url);
		open += 1;
		peak = Math.max(peak, open);
		await Promise.resolve();
		const answer = answers[url] ?? { status: 404 };
		const chunks = answer.chunks;
		const done = () => {
			open -= 1;
		};
		return {
			status: answer.status,
			ok: answer.status >= 200 && answer.status < 300,
			headers: {
				get: (name: string) =>
					name === "content-length" ? (answer.length ?? null) : null,
			},
			body: chunks
				? {
						getReader: () => {
							const queue = [...chunks];
							return {
								read: async () => {
									const next = queue.shift();
									if (next === undefined) {
										done();
										return { done: true };
									}
									return { done: false, value: new Uint8Array(next) };
								},
								cancel: async () => {
									cancelled += 1;
									done();
								},
							};
						},
					}
				: null,
			json: async () => {
				done();
				return answer.body;
			},
			arrayBuffer: async () => {
				await Promise.resolve();
				done();
				return new ArrayBuffer(answer.bytes ?? 0);
			},
		};
	};
	return {
		fetch,
		asked,
		peak: () => peak,
		cancelled: () => cancelled,
	};
}

const base = "https://api.example.com/__robledo/android/rt1";

afterEach(() => useNative(null));

async function code(run: () => Promise<unknown>): Promise<unknown> {
	try {
		await run();
	} catch (error) {
		return error instanceof Exception ? error.code : String(error);
	}
	return undefined;
}

describe("checkForUpdate", () => {
	it("has the manifest verified, then hands each file over as it arrives", async () => {
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
		expect(native.log).toEqual([
			"verify",
			"receive index.android.bundle 10",
			"receive assets/logo 1.png 3",
			"finish",
		]);
		expect(asked).toHaveLength(3);
		// Asked again: already staged, nothing downloaded.
		expect(await checkForUpdate({ fetch })).toEqual({
			kind: "staged",
			id: "u2",
		});
		expect(asked).toHaveLength(4);
	});

	it("fetches nothing a manifest names before its signature holds", async () => {
		const native = new FakeNative();
		native.refuse = "the manifest's signature does not match the app's key";
		useNative(native);
		// What a hostile server answers: thousands of files, a garbage signature.
		const many = JSON.stringify({
			id: "evil",
			files: Array.from({ length: 2000 }, (_, i) => ({
				path: `f/${i}.bin`,
				size: 65536,
			})),
		});
		const hostile = server({
			[`${base}/latest`]: {
				status: 200,
				body: { manifest: many, signature: "AAAA" },
			},
		});
		expect(await code(() => checkForUpdate({ fetch: hostile.fetch }))).toBe(
			"E_ROBLEDO_REFUSED",
		);
		expect(hostile.asked).toEqual([`${base}/latest`]);
		expect(native.log).toEqual(["verify"]);
	});

	it("fetches a few files at a time", async () => {
		const native = new FakeNative();
		useNative(native);
		const files = Array.from({ length: 20 }, (_, i) => ({
			path: `f/${i}`,
			size: 1,
		}));
		const answers: { [url: string]: Answer } = {
			[`${base}/latest`]: {
				status: 200,
				body: { manifest: JSON.stringify({ id: "u3", files }), signature: "s" },
			},
		};
		for (const file of files)
			answers[`${base}/u3/files/${file.path}`] = { status: 200, bytes: 1 };
		const many = server(answers);
		expect(await checkForUpdate({ fetch: many.fetch })).toEqual({
			kind: "staged",
			id: "u3",
		});
		expect(many.asked).toHaveLength(21);
		expect(many.peak()).toBeLessThanOrEqual(PARALLEL_DOWNLOADS);
		expect(many.peak()).toBeGreaterThan(1);
	});

	it("refuses a file larger than its signed size, and fetches nothing more", async () => {
		const answers = (file: Answer) => ({
			[`${base}/latest`]: { status: 200, body: { manifest, signature: "s" } },
			[`${base}/u2/files/index.android.bundle`]: file,
			[`${base}/u2/files/assets/logo%201.png`]: { status: 200, bytes: 3 },
		});
		// It says so up front.
		let native = new FakeNative();
		useNative(native);
		const announced = server(
			answers({ status: 200, length: "1000000000", bytes: 10 }),
		);
		expect(await code(() => checkForUpdate({ fetch: announced.fetch }))).toBe(
			"E_ROBLEDO_BAD_RESPONSE",
		);
		expect(native.log).not.toContain("finish");
		// It streams past it: cancelled at the first chunk too many.
		native = new FakeNative();
		useNative(native);
		const streamed = server(answers({ status: 200, chunks: [6, 6, 6, 6] }));
		expect(await code(() => checkForUpdate({ fetch: streamed.fetch }))).toBe(
			"E_ROBLEDO_BAD_RESPONSE",
		);
		expect(streamed.cancelled()).toBe(1);
		// No length, no stream: refused once read.
		native = new FakeNative();
		useNative(native);
		const whole = server(answers({ status: 200, bytes: 11 }));
		expect(await code(() => checkForUpdate({ fetch: whole.fetch }))).toBe(
			"E_ROBLEDO_BAD_RESPONSE",
		);
		expect(
			native.log.filter((line) => line.startsWith("receive index")),
		).toEqual([]);
	});

	it("reads a streamed file of the right size whole", async () => {
		const native = new FakeNative();
		useNative(native);
		const { fetch } = server({
			[`${base}/latest`]: { status: 200, body: { manifest, signature: "s" } },
			[`${base}/u2/files/index.android.bundle`]: {
				status: 200,
				chunks: [4, 6],
			},
			[`${base}/u2/files/assets/logo%201.png`]: {
				status: 200,
				length: "3",
				bytes: 3,
			},
		});
		expect(await checkForUpdate({ fetch })).toEqual({
			kind: "staged",
			id: "u2",
		});
		expect(native.log).toContain("receive index.android.bundle 10");
	});

	it("refuses a latest answer larger than it reads", async () => {
		useNative(new FakeNative());
		const big = server({
			[`${base}/latest`]: {
				status: 200,
				length: String(MAX_LATEST_BYTES + 1),
				body: {},
			},
		});
		expect(await code(() => checkForUpdate({ fetch: big.fetch }))).toBe(
			"E_ROBLEDO_BAD_RESPONSE",
		);
		const long = server({
			[`${base}/latest`]: {
				status: 200,
				body: { manifest: " ".repeat(MAX_LATEST_BYTES + 1), signature: "s" },
			},
		});
		expect(await code(() => checkForUpdate({ fetch: long.fetch }))).toBe(
			"E_ROBLEDO_BAD_RESPONSE",
		);
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
		expect(native.log).toEqual(["verify"]);
	});

	it("names a bad answer, a replayed update, and what the phone refused", async () => {
		const native = new FakeNative();
		useNative(native);
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
		// The files are missing on the server.
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
		const all = server({
			[`${base}/latest`]: { status: 200, body: { manifest, signature: "s" } },
			[`${base}/u2/files/index.android.bundle`]: { status: 200, bytes: 10 },
			[`${base}/u2/files/assets/logo%201.png`]: { status: 200, bytes: 3 },
		});
		native.refuse =
			"E_ROBLEDO_NOT_NEWER: update u1 was built at 1, not after 2, the newest this phone accepted";
		let thrown: unknown;
		try {
			await checkForUpdate({ fetch: all.fetch });
		} catch (error) {
			thrown = error;
		}
		expect(thrown).toBeInstanceOf(errors.E_ROBLEDO_NOT_NEWER);
		expect(Reflect.get(Object(thrown), "message")).toBe(
			"[robledo] update u1 was built at 1, not after 2, the newest this phone accepted",
		);
		native.refuse = "the manifest's signature does not match the app's key";
		expect(await code(() => checkForUpdate({ fetch: all.fetch }))).toBe(
			"E_ROBLEDO_REFUSED",
		);
		native.refuse = null;
		native.refuseFile =
			"index.android.bundle does not match its digest in the manifest";
		expect(await code(() => checkForUpdate({ fetch: all.fetch }))).toBe(
			"E_ROBLEDO_REFUSED",
		);
		// A manifest that is not even JSON is named by what it claims: nothing.
		native.refuse = "manifest: expected value";
		const garbage = server({
			[`${base}/latest`]: {
				status: 200,
				body: { manifest: "{", signature: "s" },
			},
		});
		let named: unknown;
		try {
			await checkForUpdate({ fetch: garbage.fetch });
		} catch (error) {
			named = error;
		}
		expect(Reflect.get(Object(named), "message")).toBe(
			"[robledo] update ? refused: manifest: expected value",
		);
	});
});

describe("the native module", () => {
	it("is found through React Native's registry, and its absence is named", () => {
		modules.delete(NATIVE_MODULE_NAME);
		expect(() => updateStatus()).toThrow(/TurboModuleRegistry/);
		modules.set(NATIVE_MODULE_NAME, { status: "not a function" });
		expect(() => updateStatus()).toThrow(errors.E_ROBLEDO_NO_NATIVE_MODULE);
		const native = new FakeNative();
		modules.set(NATIVE_MODULE_NAME, native);
		expect(updateStatus().runtimeVersion).toBe("rt1");
		markReady();
		expect(native.readies).toBe(1);
	});
});
