import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { runInNewContext } from "node:vm";
import ts from "typescript";
import type { StoredAuthSession } from "../auth/sessionCoordinator";
import { createSessionRefresh } from "../auth/sessionRefresh";
import { createAssistantVoiceApi } from "./assistantVoiceTransport";

function setup(
  input?:
    | { status: number; body: string; network?: boolean }
    | ((url: string, init?: RequestInit) => Promise<Response>),
  realRefresh = false,
) {
  const respond = typeof input === "function" ? input : undefined;
  const reply = typeof input === "object" ? input : undefined;
  let generation = 1;
  let invalidations = 0;
  let sends = 0;
  let commits = 0;
  let session: StoredAuthSession = {
    accessToken: "synthetic",
    refreshToken: "synthetic-refresh",
    accessTokenExpiresAt: 0,
    sessionGeneration: generation,
  };
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
    "../auth/sessionRefresh": {
      createSessionRefresh: realRefresh
        ? createSessionRefresh
        : () => async () => null,
    },
    "../auth/sessionStore": {
      authSessionGeneration: () => generation,
      loadStoredAuthSession: async () => {
        if (sessionReadGate) await sessionReadGate;
        return realRefresh ? { ...session } : { accessToken: "synthetic" };
      },
      commitRefreshedAuthSession: async (
        expected: StoredAuthSession,
        next: StoredAuthSession,
      ) => {
        if (
          expected.sessionGeneration !== generation ||
          expected.accessToken !== session.accessToken ||
          expected.refreshToken !== session.refreshToken
        )
          return false;
        commits++;
        session = { ...next, sessionGeneration: generation };
        return true;
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
      AbortController,
      setTimeout: (...args: Parameters<typeof setTimeout>) =>
        setTimeout(...args),
      clearTimeout: (timeout: ReturnType<typeof setTimeout>) =>
        clearTimeout(timeout),
      fetch: async (url: string, init?: RequestInit) => {
        sends++;
        if (respond) return respond(url, init);
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
    refresh: exports.refreshAccessTokenIfNeeded as () => Promise<boolean>,
    session: () => ({ ...session }),
    commits: () => commits,
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
        assert.equal(error.errorKey, "invalid_error_response");
        assert.equal(error.errorCode, 0);
        assert.equal(error.message, "NyxID couldn't complete this request. Please try again.");
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

async function settleMicrotasks() {
  for (let step = 0; step < 64; step++) await Promise.resolve();
}

function expectRefreshNetworkError(cause: unknown) {
  const error = cause as {
    name: string;
    statusCode: number;
    errorKey: string;
    message: string;
  };
  assert.equal(error.name, "ApiError");
  assert.equal(error.statusCode, 0);
  assert.equal(error.errorKey, "network_error");
  assert.equal(
    error.message,
    "Couldn't reach NyxID. Check your connection and try again.",
  );
  return true;
}

test("a stalled 401 refresh keeps login and never blindly replays the original POST", async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const requests: string[] = [];
  let refreshSignal: AbortSignal | null | undefined;
  const env = setup(async (url, init) => {
    requests.push(new URL(url).pathname);
    assert.equal(init?.method, "POST");
    if (url.endsWith("/auth/refresh")) {
      refreshSignal = init?.signal;
      return new Promise<Response>(() => {});
    }
    return Response.json({ error: "unauthorized" }, { status: 401 });
  }, true);
  const pending = assert.rejects(
    env.request("/assistant/nyxagent/turns", { method: "POST", body: {} }),
    expectRefreshNetworkError,
  );
  await settleMicrotasks();
  assert.equal(env.sends(), 2);
  context.mock.timers.tick(9_999);
  await settleMicrotasks();
  assert.equal(refreshSignal?.aborted, false);
  context.mock.timers.tick(1);
  await pending;
  assert.equal(refreshSignal?.aborted, true);
  assert.equal(env.invalidations(), 0);
  assert.equal(env.commits(), 0);
  assert.equal(env.session().accessToken, "synthetic");
  assert.deepEqual(requests, [
    "/api/v1/assistant/nyxagent/turns",
    "/api/v1/auth/refresh",
  ]);
});

for (const phase of ["fetch", "body"] as const) {
  test(`a stalled refresh ${phase} releases all shared callers and a late success cannot overwrite explicit retry`, async (context) => {
    context.mock.timers.enable({ apis: ["setTimeout"] });
    let releaseFetch!: (response: Response) => void;
    let releaseBody!: (body: string) => void;
    const oldFetch = new Promise<Response>((resolve) => {
      releaseFetch = resolve;
    });
    const oldBody = new Promise<string>((resolve) => {
      releaseBody = resolve;
    });
    const signals: Array<AbortSignal | null | undefined> = [];
    const env = setup(async (url, init) => {
      assert.ok(url.endsWith("/auth/refresh"));
      signals.push(init?.signal);
      if (signals.length > 1)
        return Response.json({
          access_token: "retry-access",
          refresh_token: "retry-refresh",
          expires_in: 3_600,
        });
      return phase === "fetch"
        ? oldFetch
        : ({
            status: 200,
            ok: true,
            headers: new Headers(),
            text: () => oldBody,
          } as Response);
    }, true);
    const first = assert.rejects(env.refresh(), expectRefreshNetworkError);
    const joined = assert.rejects(env.refresh(), expectRefreshNetworkError);
    await settleMicrotasks();
    assert.equal(
      env.sends(),
      1,
      "both attempts must share the pending refresh",
    );
    context.mock.timers.tick(10_000);
    await Promise.all([first, joined]);
    assert.equal(signals[0]?.aborted, true);
    assert.equal(env.invalidations(), 0);
    assert.equal(env.commits(), 0);

    assert.equal(
      await env.refresh(),
      true,
      "explicit retry must create a fresh request",
    );
    assert.equal(env.sends(), 2);
    assert.equal(env.commits(), 1);
    const current = env.session();
    assert.equal(current.accessToken, "retry-access");
    assert.equal(current.refreshToken, "retry-refresh");

    const late = {
      access_token: "late-access",
      refresh_token: "late-refresh",
      expires_in: 3_600,
    };
    releaseFetch(Response.json(late));
    releaseBody(JSON.stringify(late));
    await settleMicrotasks();
    assert.deepEqual(env.session(), current);
    assert.equal(env.commits(), 1);
    context.mock.timers.tick(10_000);
    assert.equal(
      signals[1]?.aborted,
      false,
      "successful refresh must clear its deadline",
    );
  });

  test(`a late refresh 409 from ${phase} never sends another refresh POST after the deadline`, async (context) => {
    context.mock.timers.enable({ apis: ["setTimeout"] });
    let releaseFetch!: (response: Response) => void;
    let releaseBody!: (body: string) => void;
    const oldFetch = new Promise<Response>((resolve) => {
      releaseFetch = resolve;
    });
    const oldBody = new Promise<string>((resolve) => {
      releaseBody = resolve;
    });
    const env = setup(
      async () =>
        phase === "fetch"
          ? oldFetch
          : ({
              status: 409,
              ok: false,
              headers: new Headers(),
              text: () => oldBody,
            } as Response),
      true,
    );
    const pending = assert.rejects(env.refresh(), expectRefreshNetworkError);
    await settleMicrotasks();
    context.mock.timers.tick(10_000);
    await pending;
    releaseFetch(
      Response.json({ error: "refresh_in_progress" }, { status: 409 }),
    );
    releaseBody('{"error":"refresh_in_progress"}');
    await settleMicrotasks();
    context.mock.timers.tick(160);
    await settleMicrotasks();
    assert.equal(env.sends(), 1);
    assert.equal(env.commits(), 0);
    assert.equal(env.invalidations(), 0);
  });
}

test("refresh still coordinates a 409 conflict and commits a rotating token within its deadline", async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const signals: Array<AbortSignal | null | undefined> = [];
  const env = setup(async (_url, init) => {
    signals.push(init?.signal);
    return signals.length === 1
      ? Response.json({ error: "refresh_in_progress" }, { status: 409 })
      : Response.json({
          access_token: "updated-access",
          refresh_token: "updated-refresh",
          expires_in: 3_600,
        });
  }, true);
  const pending = env.refresh();
  await settleMicrotasks();
  assert.equal(env.sends(), 1);
  context.mock.timers.tick(160);
  assert.equal(await pending, true);
  assert.equal(env.sends(), 2);
  assert.equal(env.commits(), 1);
  assert.equal(env.session().refreshToken, "updated-refresh");
  context.mock.timers.tick(10_000);
  assert.ok(signals.every((signal) => signal?.aborted === false));
});

test("an explicit server rejection of refresh retains the existing 401 sign-out behavior", async () => {
  const env = setup(
    async () => Response.json({ error: "unauthorized" }, { status: 401 }),
    true,
  );
  await assert.rejects(
    env.request("/users/me", {}),
    (error: unknown) => (error as { statusCode: number }).statusCode === 401,
  );
  assert.equal(env.sends(), 2);
  assert.equal(env.commits(), 0);
  assert.equal(env.invalidations(), 1);
});
