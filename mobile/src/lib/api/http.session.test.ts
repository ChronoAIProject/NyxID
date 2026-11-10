import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { runInNewContext } from "node:vm";
import ts from "typescript";
import { createAssistantVoiceApi } from "./assistantVoiceTransport";

function setup(
  respond?: (url: string, init?: RequestInit) => Promise<Response>,
) {
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
      fetch: async (url: string, init?: RequestInit) => {
        sends++;
        if (respond) return respond(url, init);
        return {
          status: 401,
          ok: false,
          text: async () => {
            afterRead();
            return '{"error":"invalid_mfa_code","message":"Invalid code"}';
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

for (const [status, body] of [
  [404, null],
  [405, "Method Not Allowed: synthetic upstream diagnostic"],
  [404, "<html>synthetic proxy diagnostic</html>"],
] as const) {
  test(`raw HTTP ${status} keeps status and sanitizes ${body ? "non-JSON" : "empty"} failures`, async () => {
    const env = setup(async () => new Response(body, { status }));
    await assert.rejects(
      env.request("/optional-route", {}),
      (cause: unknown) => {
        const error = cause as {
          name: string;
          statusCode: number;
          errorKey: string;
          errorCode: number;
          message: string;
        };
        assert.equal(error.name, "ApiError");
        assert.equal(error.statusCode, status);
        assert.equal(error.errorKey, `request_failed_${status}`);
        assert.equal(error.errorCode, 0);
        assert.equal(error.message, `request_failed_${status}`);
        assert.doesNotMatch(error.message, /synthetic|html|upstream/);
        return true;
      },
    );
    assert.equal(env.sends(), 1);
    assert.equal(env.invalidations(), 0);
  });

  test(`raw HTTP ${status} from an undeployed account route reaches bounded voice fallback`, async () => {
    const thread = `nyxa-${"a".repeat(32)}`;
    const paths: string[] = [];
    const env = setup(async (url, init) => {
      const path =
        new URL(url).pathname.replace("/api/v1", "") + new URL(url).search;
      paths.push(path);
      assert.equal(init?.method, "GET");
      if (path === "/assistant/nyxagent/voice/active")
        return new Response(body, { status });
      if (path.endsWith("/voice-sessions/active"))
        return Response.json({ session: null });
      assert.equal(path, "/assistant/nyxagent/conversations?limit=10");
      return Response.json({ conversations: [], next_cursor: null });
    });
    const api = createAssistantVoiceApi(
      (path, options) => env.request(path, { ...options }),
      "https://nyx.example",
    );
    assert.deepEqual(await api.accountActive(thread), {
      session: null,
      conversation_id: null,
    });
    assert.deepEqual(paths, [
      "/assistant/nyxagent/voice/active",
      `/assistant/nyxagent/conversations/${thread}/voice-sessions/active`,
      "/assistant/nyxagent/conversations?limit=10",
    ]);
    assert.equal(env.sends(), 3);
    assert.equal(env.invalidations(), 0);
  });
}

for (const status of [401, 403, 503]) {
  test(`raw HTTP ${status} must not trigger account voice fallback`, async () => {
    const paths: string[] = [];
    const env = setup(async (url) => {
      paths.push(new URL(url).pathname);
      return new Response(null, { status });
    });
    const api = createAssistantVoiceApi(
      (path, options) => env.request(path, { ...options }),
      "https://nyx.example",
    );
    await assert.rejects(
      api.accountActive(`nyxa-${"a".repeat(32)}`),
      (error: unknown) =>
        (error as { statusCode: number }).statusCode === status,
    );
    assert.deepEqual(paths, ["/api/v1/assistant/nyxagent/voice/active"]);
    assert.equal(env.invalidations(), status === 401 ? 1 : 0);
  });
}
