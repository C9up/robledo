import { readFile } from "node:fs/promises";
import { resolve, sep } from "node:path";
import { RobledoError } from "../errors.js";

/**
 * robledo on the server: it serves the updates `ream android:publish` signed
 * and wrote under the app's `robledo/` directory. It signs nothing and holds
 * no key — what reaches a phone is checked there against the key built into
 * the app, so a compromised server can withhold an update but not forge one.
 *
 *     robledo/<platform>/<runtime>/latest              the newest update's id
 *     robledo/<platform>/<runtime>/<id>/manifest.json  what the update holds
 *     robledo/<platform>/<runtime>/<id>/manifest.sig   its Ed25519 signature
 *     robledo/<platform>/<runtime>/<id>/files/…        the files it lists
 *
 * An update is per runtime: a phone asks for the one built for the native
 * code it carries, and never gets JS that needs native code it lacks.
 */
export interface RobledoConfig {
	/** Where the published updates are, from the app's root. Default `robledo`. */
	directory?: string;
	/** The routes' prefix. Default `/__robledo`. */
	prefix?: string;
}

export interface LatestUpdate {
	/** The manifest's exact text: the signature covers these bytes. */
	manifest: string;
	signature: string;
}

const PLATFORMS = new Set(["android", "ios"]);
const NAME = /^[A-Za-z0-9_-]{1,64}$/;

/** A path segment from a URL that names a file inside a bundle. */
function safeSegment(segment: string): boolean {
	return (
		segment !== "" &&
		segment !== "." &&
		segment !== ".." &&
		!segment.includes("\\") &&
		!segment.includes("\0") &&
		!segment.includes("/")
	);
}

export class Robledo {
	readonly directory: string;
	readonly prefix: string;
	#mounter: ((robledo: Robledo) => void) | null = null;
	#registered = false;

	/** `resolvePath` resolves against the app's root (ream's `app.makePath`). */
	constructor(
		config: RobledoConfig = {},
		resolvePath: (path: string) => string = (path) => resolve(path),
	) {
		this.directory = resolvePath(config.directory ?? "robledo");
		const prefix = config.prefix ?? "/__robledo";
		if (!prefix.startsWith("/") || prefix.endsWith("/")) {
			throw new RobledoError(
				"E_ROBLEDO_INVALID_PREFIX",
				`[robledo] prefix ${JSON.stringify(prefix)} must start with / and not end with one`,
			);
		}
		this.prefix = prefix;
	}

	#runtimeDir(platform: string, runtime: string): string | null {
		if (!PLATFORMS.has(platform) || !NAME.test(runtime)) return null;
		return resolve(this.directory, platform, runtime);
	}

	/** The newest update published for this platform and runtime, or null. */
	async latest(
		platform: string,
		runtime: string,
	): Promise<LatestUpdate | null> {
		const dir = this.#runtimeDir(platform, runtime);
		if (dir === null) return null;
		try {
			const id = (await readFile(resolve(dir, "latest"), "utf8")).trim();
			if (!NAME.test(id)) return null;
			const [manifest, signature] = await Promise.all([
				readFile(resolve(dir, id, "manifest.json"), "utf8"),
				readFile(resolve(dir, id, "manifest.sig"), "utf8"),
			]);
			return { manifest, signature: signature.trim() };
		} catch (error) {
			if (isMissing(error)) return null;
			throw error;
		}
	}

	/**
	 * Where a file of an update is on disk, or null for any path that is not
	 * one: an unknown platform, a malformed runtime or id, or a segment that
	 * would leave the update's `files/`.
	 */
	filePath(
		platform: string,
		runtime: string,
		id: string,
		segments: readonly string[],
	): string | null {
		const dir = this.#runtimeDir(platform, runtime);
		if (dir === null || !NAME.test(id)) return null;
		if (segments.length === 0 || !segments.every(safeSegment)) return null;
		const root = resolve(dir, id, "files");
		const path = resolve(root, ...segments);
		return path.startsWith(root + sep) ? path : null;
	}

	/** @internal The provider hands over what puts routes on the host router. */
	useMounter(mounter: (robledo: Robledo) => void): void {
		this.#mounter = mounter;
		if (this.#registered) mounter(this);
	}

	/**
	 * Put robledo's two routes on the router — from a preload, so they exist
	 * before the server listens:
	 *
	 *     GET <prefix>/:platform/:runtime/latest          the newest update (204: none)
	 *     GET <prefix>/:platform/:runtime/:id/files/*     one of its files
	 */
	registerRoutes(): void {
		if (this.#registered) return;
		this.#registered = true;
		this.#mounter?.(this);
	}

	hasRegisteredRoutes(): boolean {
		return this.#registered;
	}
}

function isMissing(error: unknown): boolean {
	return (
		typeof error === "object" &&
		error !== null &&
		(Reflect.get(error, "code") === "ENOENT" ||
			Reflect.get(error, "code") === "ENOTDIR")
	);
}
