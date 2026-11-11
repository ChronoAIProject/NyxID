import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { runInNewContext } from "node:vm";
import ts from "typescript";

function setup(reply?: { status: number; body: string; network?: boolean }) {
  let generation = 1;
  let invalidations = 0;
  let sends = 0;
  let sessionReadGate: Promise<void> | null = null;
  let afterRead = () => {};
  const filename = fileURLToPath(new URL("./http.ts", import.meta.url));
  const localRequire = createRequire(filename);
  const mocks: Record<string, unknown> = {
    "./baseUrl": {
      buildApiUrl: (path: string) => `https://api.test/api/v1${path}`,
    },
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
      loadStoredAuthSession: async () => {
        if (sessionReadGate) await sessionReadGate;
        return { accessToken: "synthetic" };
      },
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
      Headers,
      fetch: async () => {
        sends++;
        if (reply?.network) throw new TypeError("Network request failed: private URL");
        return {
          headers: new Headers({ "x-request-id": "request-2275", "X-NyxID-Trace-Id": "trace-2275" }),
          status: reply?.status ?? 401,
          ok: reply ? reply.status >= 200 && reply.status < 300 : false,
          text: async () => {
            afterRead();
            return reply?.body ?? '{"error":"invalid_mfa_code","message":"Invalid code"}';
          },
        };
      },
    },
    { filename },
  );
  return {
    request: exports.requestJson as (
      path: string,
      options: Record<string, unknown>,
    ) => Promise<unknown>,
    invalidations: () => invalidations,
    sends: () => sends,
    blockSessionRead: () => {
      let release!: () => void;
      sessionReadGate = new Promise<void>((resolve) => {
        release = resolve;
      });
      return release;
    },
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

test("JSON authority guard cancels queued writes after the screen leaves and returns", async () => {
  const env = setup();
  const release = env.blockSessionRead();
  let activation = 1;
  const expected = activation;
  const pending = env.request("/auth/password/change", {
    method: "POST",
    body: {
      current_password: "synthetic old",
      new_password: "synthetic replacement",
    },
    authorityGuard: () => {
      if (activation !== expected)
        throw Object.assign(new Error("Account action cancelled"), {
          name: "AbortError",
        });
    },
  });
  activation++;
  release();
  await assert.rejects(pending, { name: "AbortError" });
  assert.equal(env.sends(), 0, "the password body must never reach fetch");
  assert.equal(env.invalidations(), 0);
});

for (const [status, body] of [[503, "<html>nginx</html>"], [502, "private upstream"], [504, ""], [503, '{"error":']] as const) {
  test(`real JSON client sanitizes HTTP ${status} and preserves correlation`, async () => {
    const env = setup({ status, body });
    await assert.rejects(env.request("/assistant/nyxagent/agents", {}), (error: any) => {
      assert.equal(error.message, "NyxID is temporarily unavailable. Try again in a moment.");
      assert.equal(error.statusCode, status);
      assert.equal(error.requestId, "request-2275");
      assert.equal(error.traceId, "trace-2275");
      return true;
    });
    assert.equal(env.sends(), 1);
    assert.equal(env.invalidations(), 0);
  });
}
test("real JSON client maps network failure without retrying a send", async () => {
  const env = setup({ status: 0, body: "", network: true });
  await assert.rejects(env.request("/assistant/nyxagent/turns", { method: "POST", body: {} }),
    /Couldn't reach NyxID. Check your connection and try again./);
  assert.equal(env.sends(), 1);
});

test("malformed successful JSON is safe and still honors a replacement session", async () => {
  const reply = { status: 200, body: "<html>private proxy page</html>" };
  await assert.rejects(setup(reply).request("/users/me", {}),
    /NyxID couldn't complete this request. Please try again./);
  const env = setup(reply);
  env.switchDuringRead();
  await assert.rejects(env.request("/users/me", {}), /auth_session_changed/);
  assert.equal(env.invalidations(), 0);
});
