/**
 * `@c9up/robledo` — over-the-air updates for a ream app's mobile build.
 *
 * - The server serves what `ream android:publish` signed (`./provider`,
 *   `./services/main`).
 * - The app checks, downloads and stages updates (`./mobile`); the native
 *   part verifies each against the key built into the app, switches at the
 *   next start, and rolls back one that never reported ready.
 */
export * as errors from "./errors.js";
export {
	type LatestUpdate,
	Robledo,
	type RobledoConfig,
} from "./server/Robledo.js";
export {
	type RobledoHttpContext,
	type RobledoRouter,
	registerRobledoRoutes,
} from "./server/routes.js";
