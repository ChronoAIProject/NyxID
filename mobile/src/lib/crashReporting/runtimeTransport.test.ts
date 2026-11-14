import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { hookHarness } from "../../features/assistant/assistantHookTestHarness";
import type { RelayRequest } from "../analyticsRelay";
import { executeAuthenticatedRequest } from "../api/authenticatedTransport";
import { buildApiError } from "../api/responseError";

test("diagnostics transport pauses on proxy 401 without refreshing or invalidating the session", async () => {
  let request!: RelayRequest;
  let refreshes = 0;
  let invalidations = 0;
  let sends = 0;
  const h = hookHarness<void>({
    expo: { requireOptionalNativeModule: () => ({}) },
    "expo-crypto": { randomUUID: () => "uuid" },
    "expo-constants": { expoConfig: { extra: {} } },
    "react-native": { Platform: { OS: "ios" } },
    "../analyticsConsentState": { analyticsConsentState: { getSnapshot() {} } },
    "./lifecycle": {
      createCrashLifecycle: (deps: { request: RelayRequest }) => {
        request = deps.request;
        return {};
      },
    },
    "./globalHandler": {},
    "../api/http": {
      requestJson: async (path: string, options: Parameters<RelayRequest>[1]) =>
        executeAuthenticatedRequest({
          url: `https://api.test${path}`,
          options: options!,
          clientHeaders: {},
          loadSession: async () => ({ accessToken: "synthetic" }),
          transport: async () => {
            sends++;
            return {
              ok: false,
              status: 401,
              text: async () => "<html>captive portal</html>",
            };
          },
          refresh: async () => { refreshes++; return null; },
          invalidate: async () => { invalidations++; },
          toError: buildApiError,
        }),
    },
  });
  h.load(fileURLToPath(new URL("./runtime.ts", import.meta.url)));
  for (const path of ["/users/me/preferences/analytics", "/users/me/crash-reports"]) {
    await assert.rejects(request(path), (error: unknown) => {
      const refusal = error as ReturnType<typeof buildApiError>;
      assert.equal(refusal.statusCode, 401);
      assert.equal(refusal.hasErrorEnvelope, false);
      return true;
    });
  }
  assert.equal(sends, 2);
  assert.equal(refreshes, 0);
  assert.equal(invalidations, 0);
});
