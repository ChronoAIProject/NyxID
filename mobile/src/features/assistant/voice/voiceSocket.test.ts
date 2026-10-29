import assert from "node:assert/strict";
import test from "node:test";
import type { VoiceControlSocket } from "./AssistantVoiceClient";
import { createVoiceSocketFactory } from "./voiceSocket";

const thread = `nyxa-${"a".repeat(32)}`;
const sessionId = "11111111-1111-4111-8111-111111111111";
const path = `/assistant/nyxagent/conversations/${thread}/voice-sessions/${sessionId}/stream`;
type Input = Parameters<typeof createVoiceSocketFactory>[0];
function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((done, fail) => {
    resolve = done;
    reject = fail;
  });
  return { promise, resolve, reject };
}
const flush = () => new Promise<void>((resolve) => setImmediate(resolve));

function fixture(overrides: Partial<Input> = {}) {
  let epoch = 7;
  let refreshes = 0;
  let loads = 0;
  const opened: {
    url: string;
    protocols: string[] | undefined;
    options: { headers: Record<string, string> };
    socket: VoiceControlSocket;
  }[] = [];
  class Socket implements VoiceControlSocket {
    readyState = 0;
    bufferedAmount = 0;
    binaryType = "blob";
    onmessage: VoiceControlSocket["onmessage"] = null;
    onclose: VoiceControlSocket["onclose"] = null;
    onerror: VoiceControlSocket["onerror"] = null;
    send() {}
    close() {}
    constructor(
      url: string,
      protocols: string[] | undefined,
      options: { headers: Record<string, string> },
    ) {
      opened.push({ url, protocols, options, socket: this });
    }
  }
  const connect = createVoiceSocketFactory({
    apiBaseUrl: "https://api.nyx.example/api/v1/",
    origin: "https://nyx.example",
    epoch,
    getEpoch: () => epoch,
    refresh: async () => {
      refreshes++;
    },
    loadSession: async () => {
      loads++;
      return { accessToken: "human-session-token", userId: sessionId };
    },
    Socket,
    ...overrides,
  });
  return {
    connect,
    opened,
    setEpoch(value: number) {
      epoch = value;
    },
    refreshes: () => refreshes,
    loads: () => loads,
  };
}

test("native control WebSocket keeps the bearer in headers and targets only the configured API", async () => {
  const f = fixture();
  const result = await f.connect(path, new AbortController().signal);
  assert.equal(f.opened[0]?.socket, result);
  assert.equal(f.opened[0]?.url, `wss://api.nyx.example/api/v1${path}`);
  assert.deepEqual(f.opened[0]?.options.headers, {
    Authorization: "Bearer human-session-token",
    Origin: "https://nyx.example",
    "X-NyxID-Client": "mobile",
  });
  assert.equal(f.opened[0]?.protocols, undefined);
  assert.doesNotMatch(
    f.opened[0]!.url,
    /human-session-token|Bearer|token=|authorization/i,
  );
  assert.equal(f.refreshes(), 1);
  assert.equal(f.loads(), 1);
});

test("configured HTTP development endpoints use ws and preserve the API prefix", async () => {
  const f = fixture({ apiBaseUrl: "http://127.0.0.1:3001/api/v1" });
  await f.connect(path, new AbortController().signal);
  assert.equal(f.opened[0]?.url, `ws://127.0.0.1:3001/api/v1${path}`);
});

test("invalid stream paths cannot redirect credentials or trigger refresh", async () => {
  for (const invalid of [
    "https://evil.example/stream",
    "//evil.example/stream",
    "/assistant/nyxagent/voice/options",
    path.replace(thread, "../another-thread"),
    path.replace(sessionId, "-".repeat(36)),
    path.replace(sessionId, "a".repeat(36)),
    `${path}?access_token=secret`,
    `${path}#fragment`,
    `${path}/../control`,
  ]) {
    const f = fixture();
    await assert.rejects(f.connect(invalid, new AbortController().signal));
    assert.equal(f.refreshes(), 0);
    assert.equal(f.opened.length, 0);
  }
});

test("invalid API destinations are rejected before credentials are loaded", async () => {
  for (const apiBaseUrl of [
    "file:///tmp/voice",
    "wss://api.nyx.example/api/v1",
    "https://user:password@api.nyx.example/api/v1",
    "https://api.nyx.example/api/v1?token=secret",
    "https://api.nyx.example/api/v1#fragment",
    "not-a-url",
  ]) {
    const f = fixture({ apiBaseUrl });
    await assert.rejects(f.connect(path, new AbortController().signal));
    assert.equal(f.refreshes(), 0);
    assert.equal(f.loads(), 0);
    assert.equal(f.opened.length, 0);
  }
});

test("only a configured trusted origin can enter native WebSocket headers", async () => {
  for (const origin of [
    "null",
    "file:///voice.html",
    "https://user:pass@nyx.example",
    "https://nyx.example/chat",
    "https://nyx.example?token=secret",
    "https://nyx.example#fragment",
    "https://nyx.example\nInjected: header",
  ]) {
    let f: ReturnType<typeof fixture> | undefined;
    await assert.rejects(async () => {
      f = fixture({ origin });
      await f.connect(path, new AbortController().signal);
    });
    assert.equal(f?.opened.length ?? 0, 0);
  }
});

test("an already cancelled or different-account connection performs no asynchronous work", async () => {
  const aborted = fixture();
  const signal = new AbortController();
  signal.abort();
  await assert.rejects(aborted.connect(path, signal.signal), {
    name: "AbortError",
  });
  assert.equal(aborted.refreshes(), 0);
  assert.equal(aborted.loads(), 0);
  const changed = fixture();
  changed.setEpoch(8);
  await assert.rejects(changed.connect(path, new AbortController().signal), {
    name: "AbortError",
  });
  assert.equal(changed.refreshes(), 0);
  assert.equal(changed.loads(), 0);
  assert.equal(changed.opened.length, 0);
});

test("cancelling while token refresh waits never loads the session or opens a socket", async () => {
  const refresh = deferred<void>();
  const f = fixture({ refresh: () => refresh.promise });
  const signal = new AbortController();
  const result = f.connect(path, signal.signal);
  signal.abort();
  refresh.resolve();
  await assert.rejects(result, { name: "AbortError" });
  assert.equal(f.loads(), 0);
  assert.equal(f.opened.length, 0);
});

test("account changes during token refresh cannot open a socket for the new account", async () => {
  const refresh = deferred<void>();
  const f = fixture({ refresh: () => refresh.promise });
  const result = f.connect(path, new AbortController().signal);
  f.setEpoch(8);
  refresh.resolve();
  await assert.rejects(result, { name: "AbortError" });
  assert.equal(f.loads(), 0);
  assert.equal(f.opened.length, 0);
});

test("cancelling while secure session loading waits prevents socket creation", async () => {
  const session = deferred<{ accessToken: string }>();
  const f = fixture({ loadSession: () => session.promise });
  const signal = new AbortController();
  const result = f.connect(path, signal.signal);
  await flush();
  signal.abort();
  session.resolve({ accessToken: "late-token" });
  await assert.rejects(result, { name: "AbortError" });
  assert.equal(f.opened.length, 0);
});

test("account changes while secure session loading waits never expose the late token", async () => {
  const session = deferred<{ accessToken: string }>();
  const f = fixture({ loadSession: () => session.promise });
  const result = f.connect(path, new AbortController().signal);
  await flush();
  f.setEpoch(8);
  session.resolve({ accessToken: "new-account-token" });
  await assert.rejects(result, { name: "AbortError" });
  assert.equal(f.opened.length, 0);
});

test("missing human tokens and refresh/storage failures do not fall back to unauthenticated media", async () => {
  for (const loadSession of [
    async () => null,
    async () => ({ accessToken: "" }),
  ]) {
    const f = fixture({ loadSession });
    await assert.rejects(
      f.connect(path, new AbortController().signal),
      /Sign in again/,
    );
    assert.equal(f.opened.length, 0);
  }
  const denied = new Error("refresh session revoked");
  const refreshFailure = fixture({
    refresh: async () => {
      throw denied;
    },
  });
  await assert.rejects(
    refreshFailure.connect(path, new AbortController().signal),
    (error) => error === denied,
  );
  assert.equal(refreshFailure.loads(), 0);
  assert.equal(refreshFailure.opened.length, 0);
  const storageFailure = fixture({
    loadSession: async () => {
      throw new Error("secure storage unavailable");
    },
  });
  await assert.rejects(
    storageFailure.connect(path, new AbortController().signal),
    /secure storage unavailable/,
  );
  assert.equal(storageFailure.opened.length, 0);
});

test("dictation uses the same authenticated native socket with its exact dedicated path", async () => {
  const f = fixture();
  const dictation = `/assistant/nyxagent/conversations/${thread}/dictation`;
  await f.connect(dictation, new AbortController().signal);
  assert.equal(f.opened[0]?.url, `wss://api.nyx.example/api/v1${dictation}`);
  assert.equal(f.opened[0]?.options.headers.Origin, "https://nyx.example");
  for (const invalid of [
    `${dictation}?model=other`,
    dictation.replace(thread, "../other"),
    `${dictation}/stream`,
  ])
    await assert.rejects(f.connect(invalid, new AbortController().signal));
});
