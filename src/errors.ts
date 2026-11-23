/**
 * robledo's errors, one per code: every failure carries a stable
 * `E_ROBLEDO_<REASON>` code, so a caller branches on `error.code` or on
 * `instanceof errors.E_ROBLEDO_…`.
 */
import { createError } from "@c9up/flare";

/** The routes' prefix does not start with `/`, or ends with one. */
export const E_ROBLEDO_INVALID_PREFIX = createError<[string]>(
	"[robledo] prefix %j must start with / and not end with one",
	"E_ROBLEDO_INVALID_PREFIX",
);

/** `services/main` used before the provider booted. */
export const E_ROBLEDO_NOT_BOOTED = createError(
	"[robledo] not booted: add @c9up/robledo/provider to reamrc.ts's providers",
	"E_ROBLEDO_NOT_BOOTED",
);

/** The app has no native robledo module: it was not built with robledo installed. */
export const E_ROBLEDO_NO_NATIVE_MODULE = createError(
	"[robledo] the NativeRobledo module is missing: build the app with @c9up/robledo installed (ream android:build)",
	"E_ROBLEDO_NO_NATIVE_MODULE",
);

/** The server answered an update check with something other than an update. */
export const E_ROBLEDO_BAD_RESPONSE = createError<[string]>(
	"[robledo] %s",
	"E_ROBLEDO_BAD_RESPONSE",
);

/** The phone refused an update: bad signature, altered file, wrong runtime… */
export const E_ROBLEDO_REFUSED = createError<[string, string]>(
	"[robledo] update %s refused: %s",
	"E_ROBLEDO_REFUSED",
);
