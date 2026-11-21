/**
 * robledo's errors: every failure carries a stable `code`
 * (`E_ROBLEDO_<REASON>`), so a caller branches on what went wrong.
 */
export type RobledoErrorCode =
	/** The routes' prefix does not start with `/`, or ends with one. */
	| "E_ROBLEDO_INVALID_PREFIX"
	/** `services/main` used before the provider booted. */
	| "E_ROBLEDO_NOT_BOOTED"
	/** The app has no native robledo module: it was not built with robledo installed. */
	| "E_ROBLEDO_NO_NATIVE_MODULE"
	/** The server answered an update check with something other than an update. */
	| "E_ROBLEDO_BAD_RESPONSE"
	/** The phone refused an update: bad signature, altered file, wrong runtime… */
	| "E_ROBLEDO_REFUSED";

export class RobledoError extends Error {
	readonly code: RobledoErrorCode;

	constructor(code: RobledoErrorCode, message: string, options?: ErrorOptions) {
		super(message, options);
		this.name = new.target.name;
		this.code = code;
	}
}
