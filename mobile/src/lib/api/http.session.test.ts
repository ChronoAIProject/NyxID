import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { runInNewContext } from "node:vm";
import ts from "typescript";

function setup() {
  let generation = 1;
  let invalidations = 0;
  let afterRead = () => {};
  const filename = fileURLToPath(new URL("./http.ts", import.meta.url));
  const localRequire = createRequire(filename);
  const mocks: Record<string, unknown> = {
    "./baseUrl": { buildApiUrl: (path: string) => `https://api.test/api/v1${path}` },
    "expo-constants": {
      __esModule: true,
      default: { expoConfig: { version: "test" } },
    },
    "expo/fetch": {
      fetch: () => {
        throw new Error("JSON must use its native JSON transport");
      },
    },
    "../auth/sessionRefresh": { createSessionRefresh: () => async () => null },
    "../auth/sessionStore": {
      authSessionGeneration: () => generation,
      loadStoredAuthSession: async () => ({ accessToken: "synthetic" }),
      clearStoredAuthSession: async () => {
        invalidations++;
      },
    },
    "../analyticsConsentState": {
      analyticsDeviceHeaders: () => ({}),
      analyticsConsentState: { getSnapshot: () => ({ accountId: "actor" }) },
    },
  };
  const exports: Record<string, unknown> = {};
  runInNewContext(
    ts.transpileModule(readFileSync(filename, "utf8"), {
      compilerOptions: {
        module: ts.ModuleKind.CommonJS,
        target: ts.ScriptTarget.ES2022,
        esModuleInterop: true,
      },
    }).outputText,
    {
      exports,
      require: (name: string) =>
        name in mocks ? mocks[name] : localRequire(name),
      process: { env: {} },
      __DEV__: false,
      fetch: async () => ({
        status: 401,
        ok: false,
        text: async () => {
          afterRead();
          return '{"error":"invalid_mfa_code","message":"Invalid code"}';
        },
      }),
    },
    { filename },
  );
  return {
    request: exports.requestJson as (
      path: string,
      options: Record<string, unknown>,
    ) => Promise<unknown>,
    invalidations: () => invalidations,
    switchDuringRead: () => {
      afterRead = () => {
        generation++;
      };
    },
  };
}

test("real JSON request options preserve a signed-in MFA session on bad-code 401", async () => {
  const env = setup();
  await assert.rejects(
    env.request("/auth/mfa/activate", {
      method: "POST",
      retryOnAuthFailure: false,
      invalidateSessionOnAuthFailure: false,
    }),
    /Invalid code/,
  );
  assert.equal(env.invalidations(), 0);
});
test("default JSON 401 still invalidates the current session", async () => {
  const env = setup();
  await assert.rejects(
    env.request("/users/me", { retryOnAuthFailure: false }),
    /Invalid code/,
  );
  assert.equal(env.invalidations(), 1);
});
test("a late JSON 401 never invalidates a replacement session for either option", async () => {
  for (const invalidateSessionOnAuthFailure of [false, true]) {
    const env = setup();
    env.switchDuringRead();
    await assert.rejects(
      env.request("/auth/mfa/activate", {
        retryOnAuthFailure: false,
        invalidateSessionOnAuthFailure,
      }),
      /auth_session_changed/,
    );
    assert.equal(env.invalidations(), 0);
  }
});
