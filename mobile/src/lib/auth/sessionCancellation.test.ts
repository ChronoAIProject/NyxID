import assert from "node:assert/strict";
import test from "node:test";
import { createSessionRefresh } from "./sessionRefresh";
import { resolveErrorMessage } from "../api/errorMessages";

for (const response of [null, { access_token: "new", refresh_token: "new-refresh", expires_in: 60 }]) {
  test(`N5 replaced refresh ${response ? "success" : "refusal"} is a presentable cancellation`, async () => {
    let loads = 0;
    const refresh = createSessionRefresh({
      load: async () => ({ accessToken: ++loads === 1 ? "old" : "replacement", refreshToken: "refresh", sessionGeneration: loads }),
      request: async () => response,
      commit: async () => false,
      expiresAt: (value) => value,
    });
    await assert.rejects(refresh(), (error: unknown) => {
      assert.ok(error instanceof Error);
      assert.equal(error.name, "AbortError");
      assert.equal(resolveErrorMessage(error), "Request cancelled");
      return true;
    });
  });
}
