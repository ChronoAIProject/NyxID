import assert from "node:assert/strict";
import { test } from "node:test";
import {
  executeAuthenticatedRequest,
  type RequestOptions,
} from "./authenticatedTransport";

function fixture(options: RequestOptions = {}) {
  let session: { accessToken: string; userId: string } | null = {
    accessToken: "old",
    userId: "alice",
  };
  let sends = 0;
  let refreshes = 0;
  let invalidations = 0;
  const statuses = [200];
  const input = {
    url: "https://example.test/api/v1/assistant/nyxagent/turns",
    options,
    clientHeaders: { "X-NyxID-Client": "mobile" },
    loadSession: async () => session,
    refresh: async () => {
      refreshes++;
      session = { accessToken: "new", userId: "alice" };
      return { access_token: "new" };
    },
    invalidate: async () => {
      invalidations++;
      session = null;
    },
    toError: (_: unknown, status: number) => new Error(`HTTP ${status}`),
    transport: async (
      _url: string,
      init: { headers: Record<string, string> },
    ) => {
      sends++;
      assert.equal(
        init.headers.Authorization,
        sends === 1 ? "Bearer old" : "Bearer new",
      );
      const status = statuses.shift() ?? 200;
      return {
        status,
        ok: status < 400,
        text: async () => '{"message":"unauthorized"}',
      };
    },
  };
  return {
    input,
    statuses,
    setSession: (next: typeof session) => {
      session = next;
    },
    counts: () => ({ sends, refreshes, invalidations }),
  };
}

test("explicit unauthorized admission refreshes and sends once with successor token", async () => {
  const f = fixture({ method: "POST", body: { message: "hello" } });
  f.statuses.splice(0, 1, 401, 200);
  await executeAuthenticatedRequest(f.input);
  assert.deepEqual(f.counts(), { sends: 2, refreshes: 1, invalidations: 0 });
});

test("uncertain network delivery never retries an effectful request", async () => {
  const f = fixture({ method: "POST" });
  let attempts = 0;
  f.input.transport = async () => {
    attempts++;
    throw new Error("connection lost after send");
  };
  await assert.rejects(executeAuthenticatedRequest(f.input), /connection lost/);
  assert.equal(attempts, 1);
  assert.equal(f.counts().refreshes, 0);
});

test("wrong MFA disable password does not refresh or erase the signed-in session", async () => {
  const f = fixture({
    retryOnAuthFailure: false,
    invalidateSessionOnAuthFailure: false,
  });
  f.statuses.splice(0, 1, 401);
  await assert.rejects(executeAuthenticatedRequest(f.input), /HTTP 401/);
  assert.deepEqual(f.counts(), { sends: 1, refreshes: 0, invalidations: 0 });
});

test("late response from previous account cannot expose data or invalidate new account", async () => {
  const f = fixture();
  f.input.transport = async () => {
    f.setSession({ accessToken: "bob-token", userId: "bob" });
    return { status: 401, ok: false, text: async () => "old account" };
  };
  await assert.rejects(executeAuthenticatedRequest(f.input), {
    name: "AbortError",
    message: "auth_session_changed",
  });
  assert.deepEqual(f.counts(), { sends: 0, refreshes: 0, invalidations: 0 });
});

test("cancelled request does not reach transport", async () => {
  const controller = new AbortController();
  controller.abort();
  const f = fixture({ signal: controller.signal });
  await assert.rejects(executeAuthenticatedRequest(f.input), {
    name: "AbortError",
  });
  assert.equal(f.counts().sends, 0);
});

test("cancellation while secure session load waits prevents dispatch", async () => {
  const controller = new AbortController();
  const f = fixture({ method: "POST", signal: controller.signal });
  let release!: () => void;
  const pendingLoad = new Promise<void>((resolve) => {
    release = resolve;
  });
  const loadSession = f.input.loadSession;
  f.input.loadSession = async () => {
    await pendingLoad;
    return loadSession();
  };
  const request = executeAuthenticatedRequest(f.input);
  controller.abort();
  release();
  await assert.rejects(request, { name: "AbortError" });
  assert.deepEqual(f.counts(), { sends: 0, refreshes: 0, invalidations: 0 });
});

test("account switch while secure session load waits prevents dispatch", async () => {
  const f = fixture({ method: "DELETE" });
  let epoch = 0;
  let release!: () => void;
  const pendingLoad = new Promise<void>((resolve) => {
    release = resolve;
  });
  const loadSession = f.input.loadSession;
  f.input.loadSession = async () => {
    await pendingLoad;
    return loadSession();
  };
  const request = executeAuthenticatedRequest({
    ...f.input,
    getAuthEpoch: () => epoch,
  });
  f.setSession({ accessToken: "bob-token", userId: "bob" });
  epoch++;
  release();
  await assert.rejects(request, {
    name: "AbortError",
    message: "auth_session_changed",
  });
  assert.deepEqual(f.counts(), { sends: 0, refreshes: 0, invalidations: 0 });
});

test("successful body completing after an account switch cannot be consumed", async () => {
  const f = fixture();
  const { response, assertOwner } = await executeAuthenticatedRequest(f.input);
  f.setSession({ accessToken: "bob-token", userId: "bob" });
  await response.text();
  await assert.rejects(assertOwner(), { name: "AbortError" });
});

test("same-account sign-out and sign-in rejects an earlier session's response", async () => {
  const f = fixture();
  let epoch = 0;
  const { assertOwner } = await executeAuthenticatedRequest({
    ...f.input,
    getAuthEpoch: () => epoch,
  });
  epoch++;
  await assert.rejects(assertOwner(), { name: "AbortError" });
});

test("raw attachment bytes preserve binary content and headers without JSON serialization", async () => {
  const bytes = new Uint8Array([0, 255, 13, 10, 195, 169]).buffer;
  const f = fixture({
    method: "POST",
    rawBody: bytes,
    headers: {
      "Content-Type": "application/octet-stream",
      "X-Attachment-Name": "check.txt",
    },
  });
  const transport = async (
    _url: string,
    init: { headers: Record<string, string>; body?: string | ArrayBuffer },
  ) => {
    assert.equal(init.body, bytes);
    assert.equal(init.headers["Content-Type"], "application/octet-stream");
    assert.equal(init.headers["X-Attachment-Name"], "check.txt");
    return { status: 200, ok: true, text: async () => "{}" };
  };
  await executeAuthenticatedRequest({ ...f.input, transport });
});
test("mixed JSON and raw body are refused before session/transport dispatch", async () => {
  const f = fixture({ body: {}, rawBody: new ArrayBuffer(1) });
  await assert.rejects(
    executeAuthenticatedRequest(f.input),
    /either JSON or raw/,
  );
  assert.equal(f.counts().sends, 0);
});


test("authority revoked during a session read is fenced before returning a response", async () => {
  let admitted = true;
  const f = fixture({ authorityGuard: () => { if (!admitted) throw new Error("authority revoked"); } });
  const load = f.input.loadSession;
  let loads = 0;
  f.input.loadSession = async () => {
    const value = await load();
    if (++loads === 3) admitted = false;
    return value;
  };
  await assert.rejects(executeAuthenticatedRequest(f.input), /authority revoked/);
  assert.equal(f.counts().sends, 1);
});
