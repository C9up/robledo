import { Exception } from "@c9up/flare";
import { describe, expect, it } from "vitest";
import { errors } from "../../src/index.js";
import { errors as mobileErrors } from "../../src/mobile/index.js";

describe("robledo > errors", () => {
	it("exports one namespace, from the server entry and the mobile one", () => {
		expect(mobileErrors).toBe(errors);
		const error = new errors.E_ROBLEDO_INVALID_PREFIX(["robledo"]);
		expect(error).toBeInstanceOf(Exception);
		expect(error.code).toBe("E_ROBLEDO_INVALID_PREFIX");
		expect(error.status).toBe(500);
		expect(error.message).toBe(
			'[robledo] prefix "robledo" must start with / and not end with one',
		);
	});
});
