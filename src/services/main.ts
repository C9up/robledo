/**
 * The application's `Robledo`, as the provider bound it:
 *
 *     import robledo from "@c9up/robledo/services/main"
 *     robledo.registerRoutes()
 */
import { RobledoError } from "../errors.js";
import { Robledo } from "../server/Robledo.js";

let instance: Robledo | undefined;

/** @internal */
export function setRobledo(value: Robledo): void {
	instance = value;
}

/** @internal */
export function getRobledo(): Robledo | undefined {
	return instance;
}

/** @internal */
export function clearRobledo(): void {
	instance = undefined;
}

function current(): Robledo {
	if (instance === undefined) {
		throw new RobledoError(
			"E_ROBLEDO_NOT_BOOTED",
			"[robledo] not booted: add @c9up/robledo/provider to reamrc.ts's providers",
		);
	}
	return instance;
}

// A Proxy so the import can sit at a module's top, before the provider boots.
// It answers undefined to symbols and `then`: a module namespace being awaited
// or inspected must not read as a thenable or throw.
const robledo: Robledo = new Proxy<Robledo>(Object.create(Robledo.prototype), {
	get(_target, key) {
		if (typeof key === "symbol" || key === "then") return undefined;
		const target = current();
		const value: unknown = Reflect.get(target, key, target);
		return typeof value === "function" ? value.bind(target) : value;
	},
});

export default robledo;
