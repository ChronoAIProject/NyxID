import assert from "node:assert/strict";
import { randomUUID } from "node:crypto";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { deferred, hookHarness } from "../../features/assistant/assistantHookTestHarness";
import { createSessionRefresh } from "../auth/sessionRefresh";
import type { StoredAuthSession } from "../auth/sessionCoordinator";
import type { ConsentSnapshot } from "../analyticsConsentState";
import { createCrashLifecycle, type CrashNativeModule } from "./lifecycle";
import type { CrashBinding } from "./nativeRecord";

type Outcome = "accepted" | "refresh_denied" | "proxy" | "forbidden" | "validation";
const permissionPath = "/users/me/preferences/analytics";
const reportPath = "/users/me/crash-reports";

function diagnosticsTransport(outcome: Outcome, refusalPath = reportPath) {
  const originalBinding: CrashBinding = {
    accountId: randomUUID(), revision: randomUUID(), epochId: randomUUID(),
  };
  const snapshot: ConsentSnapshot = {
    accountId: originalBinding.accountId, revision: 1, enabled: true, asked: true,
    policy: 2, isHydrated: true, persistenceError: false,
  };
  const token = (exp: number) => `header.${Buffer.from(JSON.stringify({
    sub: originalBinding.accountId, exp,
  })).toString("base64url")}.signature`;
  let session: StoredAuthSession = {
    accessToken: token(0), refreshToken: "synthetic-refresh", sessionGeneration: 1,
  };
  const permission = {
    account_id: originalBinding.accountId, revision: originalBinding.revision,
    enabled: true, policy_version: "2", diagnostics_status: "available",
    diagnostics_available: true,
  };
  let binding: CrashBinding | null = { ...originalBinding };
  let records = [JSON.stringify({ binding, report: {
    schema_version: 1, event_id: randomUUID(), captured_at: new Date().toISOString(),
    platform: "ios", kind: "js_fatal", exception_class: "TypeError",
    app_version: "1.2.3", app_build: "45", os_version: "18.1",
    js_frames: [{ asset: "main.jsbundle", line: 12, column: 3 }],
    native_images: [], native_frames: [],
  } })];
  let refreshes = 0, invalidations = 0, purges = 0, removals = 0;
  let armed = false, refreshed = false;
  const refusals: number[] = [];
  const acceptedPost = deferred<void>();
  const finishPost = deferred<void>();
  const native: CrashNativeModule = {
    getAppIdentity: () => ({ version: "1.2.3", build: "45" }),
    getBinding: () => binding,
    arm: (value) => { binding = { ...value }; armed = true; return true; },
    pause: () => { armed = false; },
    disarm: () => { purges++; binding = null; records = []; armed = false; },
    collectPending: () => records,
    removeRecords: (ids) => {
      removals += ids.length;
      records = records.filter((raw) => !ids.includes(JSON.parse(raw).report.event_id));
    },
    recordFatalJs: () => assert.fail("a paused reporter must not capture"),
  };
  const h = hookHarness<void>({
    expo: { requireOptionalNativeModule: () => native },
    "expo-crypto": { randomUUID },
    "expo-constants": { expoConfig: { extra: {} } },
    "expo/fetch": { fetch: () => assert.fail("JSON uses its native fetch transport") },
    "react-native": { Platform: { OS: "ios" } },
    "../analyticsConsentState": {
      analyticsConsentState: { getSnapshot: () => snapshot },
      analyticsDeviceHeaders: () => ({}),
    },
    "./lifecycle": { createCrashLifecycle },
    "./globalHandler": {},
    "./baseUrl": { buildApiUrl: (path: string) => `https://api.test/api/v1${path}` },
    "../auth/sessionRefresh": { createSessionRefresh },
    "../auth/sessionStore": {
      authSessionGeneration: () => 1,
      loadStoredAuthSession: async () => ({ ...session }),
      commitRefreshedAuthSession: async (_expected: StoredAuthSession, next: StoredAuthSession) => {
        session = { ...next, sessionGeneration: 1 };
        return true;
      },
      clearStoredAuthSession: async () => { invalidations++; },
    },
  }, {
    __DEV__: false, Headers, Response, process: { env: {} },
    fetch: async (url: string, init?: RequestInit) => {
      if (url.endsWith("/auth/refresh")) {
        refreshes++;
        if (outcome !== "accepted") {
          return Response.json({ error: "unauthorized", error_code: 1000 }, { status: 401 });
        }
        refreshed = true;
        return Response.json({ access_token: token(Math.floor(Date.now() / 1000) + 900),
          refresh_token: "rotated-refresh", expires_in: 900 });
      }
      if (url.endsWith(refusalPath) && !refreshed) {
        assert.equal((init?.headers as Record<string, string>).Authorization, `Bearer ${token(0)}`);
        const status = outcome === "forbidden" ? 403 : outcome === "validation" ? 400 : 401;
        refusals.push(status);
        return outcome === "proxy"
          ? new Response("<html>captive portal</html>", { status })
          : Response.json({ error: "request_refused", error_code: 1000 }, { status });
      }
      if (url.endsWith(permissionPath)) return Response.json(permission);
      assert.ok(url.endsWith(reportPath));
      acceptedPost.resolve();
      await finishPost.promise;
      return Response.json({ status: "accepted" }, { status: 202 });
    },
  });
  const http = h.load(fileURLToPath(new URL("../api/http.ts", import.meta.url)));
  h.mock("../api/http", http);
  const { crashLifecycle } = h.load(fileURLToPath(new URL("./runtime.ts", import.meta.url))) as {
    crashLifecycle: ReturnType<typeof createCrashLifecycle>;
  };
  return {
    lifecycle: crashLifecycle, originalBinding, acceptedPost, finishPost, refusals,
    state: () => ({ binding, records, refreshes, invalidations, purges, removals, armed }),
  };
}

for (const path of [permissionPath, reportPath]) {
  test(`diagnostics expired token at ${path} refreshes and retries before removing an accepted record`, { timeout: 5000 }, async () => {
    const h = diagnosticsTransport("accepted", path);
    const refresh = h.lifecycle.setRestoring(false);
    await h.acceptedPost.promise;
    assert.equal(h.state().records.length, 1);
    assert.equal(h.state().removals, 0);
    assert.equal(h.state().refreshes, 1);
    assert.deepEqual(h.refusals, [401]);
    h.finishPost.resolve();
    await refresh;
    assert.equal(h.state().records.length, 0);
    assert.equal(h.state().removals, 1);
    assert.equal(h.state().invalidations, 0);
    assert.equal(h.state().purges, 0);
    assert.deepEqual(h.state().binding, h.originalBinding);
  });
  test(`diagnostics failed refresh at ${path} keeps the record paused and the session intact`, async () => {
    const h = diagnosticsTransport("refresh_denied", path);
    await h.lifecycle.setRestoring(false);
    assert.equal(h.state().refreshes, 1);
    assert.equal(h.state().records.length, 1);
    assert.equal(h.state().armed, false);
    assert.equal(h.state().removals, 0);
    assert.equal(h.state().invalidations, 0);
    assert.equal(h.state().purges, 0);
    assert.deepEqual(h.state().binding, h.originalBinding);
    h.lifecycle.fatal(new Error("paused"));
  });
}

test("diagnostics transport allows refresh on proxy 401 but keeps the record paused without invalidating the session", async () => {
  const h = diagnosticsTransport("proxy");
  await h.lifecycle.setRestoring(false);
  assert.equal(h.state().refreshes, 1);
  assert.equal(h.state().records.length, 1);
  assert.equal(h.state().armed, false);
  assert.equal(h.state().invalidations, 0);
  assert.equal(h.state().purges, 0);
  assert.deepEqual(h.state().binding, h.originalBinding);
});

for (const outcome of ["forbidden", "validation"] as const) {
  test(`diagnostics enveloped ${outcome} ${outcome === "forbidden" ? "keeps and pauses" : "drops only"} the buffered record`, async () => {
    const h = diagnosticsTransport(outcome);
    await h.lifecycle.setRestoring(false);
    assert.equal(h.state().records.length, outcome === "forbidden" ? 1 : 0);
    assert.equal(h.state().armed, outcome === "validation");
    assert.equal(h.state().refreshes, 0);
    assert.equal(h.state().invalidations, 0);
    assert.equal(h.state().purges, 0);
    assert.deepEqual(h.state().binding, h.originalBinding);
  });
}
