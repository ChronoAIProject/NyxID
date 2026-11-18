import assert from "node:assert/strict";
import test, { type TestContext } from "node:test";
import { getEventListeners } from "node:events";
import {
  ACCOUNT_EXPORT_CANCEL_TIMEOUT_MS,
  ACCOUNT_EXPORT_DEADLINE_MS,
  ACCOUNT_EXPORT_IDLE_TIMEOUT_MS,
  ACCOUNT_EXPORT_HEADERS_TIMEOUT_MS,
  cleanStaleAccountExports,
  shareAccountExport,
} from "./accountExport";

function fixture({ available = true, switchAccount = false, failShare = false, abort = false, length = null as string | null, encoding = null as string | null, invalid = false, body = undefined as string | undefined, pieces = undefined as Uint8Array[] | undefined } = {}) {
  const calls: string[] = [];
  let reads = 0;
  let requestSignal: AbortSignal | undefined;
  const text = body ?? (invalid ? '{"format":' : '{"format":"nyxid.account-export","note":"你好"}');
  const encoded = new TextEncoder().encode(text);
  const chunks = pieces ?? [encoded.slice(0, 5), encoded.slice(5)];
  return { calls, signal: () => requestSignal, deps: {
    available: async () => { calls.push("available"); return available; },
    request: async (signal: AbortSignal) => { requestSignal = signal; calls.push("request"); return {
      response: { headers: { get: (name: string) => name === "Content-Encoding" ? encoding : length }, body: { getReader: () => ({
        read: async () => { if (abort && reads > 0) throw new TypeError("connection aborted"); const value = chunks[reads++]; return { done: !value, value }; },
        cancel: async () => { calls.push("cancel"); }, releaseLock: () => { calls.push("release"); },
      }) } },
      assertOwner: async () => { calls.push("owner"); if (switchAccount && reads > 0) throw new Error("auth_session_changed"); },
    }; },
    create: () => { calls.push("create"); return {
      uri: "cache/export.json", write: (bytes: Uint8Array) => { calls.push(`write:${bytes.length}`); },
      text: async () => { calls.push("text"); throw new Error("Export file must never be read back"); },
      close: () => { calls.push("close"); }, remove: () => { calls.push("remove"); },
    }; },
    share: async () => { calls.push("share"); if (failShare) throw new Error("dismissed"); },
  } };
}

test("sharing availability is checked before admission and unavailable sharing makes no request", async () => {
  const { calls, deps } = fixture({ available: false });
  await assert.rejects(shareAccountExport(deps), /sharing_unavailable/);
  assert.deepEqual(calls, ["available"]);
});

test("export streams bytes with account checks and cleans the cache after a void share result", async () => {
  const { calls, deps } = fixture();
  assert.equal(await shareAccountExport(deps), undefined, "void share result does not prove the file was saved");
  assert.deepEqual(calls.filter((c) => c.startsWith("write:")), ["write:5", "write:44"]);
  assert.ok(calls.indexOf("close") < calls.indexOf("share"));
  assert.equal(calls.filter((call) => call === "remove").length, 1);
  assert.equal(calls.filter((c) => c === "owner").length, 5);
});

test("account switches and share failures close and remove the file without claiming success", async () => {
  for (const options of [{ switchAccount: true }, { failShare: true }]) {
    const { calls, deps } = fixture(options);
    await assert.rejects(shareAccountExport(deps));
    assert.equal(calls.filter((call) => call === "remove").length, 1);
    assert.equal(calls.filter((c) => c === "close").length, 1);
    if (options.switchAccount) assert.ok(!calls.includes("share"));
  }
});

test("next mount removes only stale export files left by an interrupted app", () => {
  const removed: string[] = [];
  cleanStaleAccountExports(["nyxid-account-export-killed.json", "photo.jpg", "other.json"].map((name) => ({ name, delete: () => { removed.push(name); } })));
  assert.deepEqual(removed, ["nyxid-account-export-killed.json"]);
});

test("an oversized stream is cancelled before writing beyond the server limit", async () => {
  const { calls, deps } = fixture();
  deps.request = async () => ({ response: { headers: { get: () => null }, body: { getReader: () => ({
    read: async () => ({ done: false, value: new Uint8Array(32 * 1024 * 1024 + 1) }),
    cancel: async () => { calls.push("cancel"); }, releaseLock: () => {},
  }) } }, assertOwner: async () => {} });
  await assert.rejects(shareAccountExport(deps), /export_too_large/);
  assert.ok(!calls.some((c) => c.startsWith("write:")));
  assert.equal(calls.filter((call) => call === "remove").length, 1);
});

for (const [name, options] of [
  ["aborted response", { abort: true }],
  ["short response", { length: "1024" }],
  ["incomplete JSON without Content-Length", { invalid: true }],
] as const) {
  test(`${name} removes the partial file and never shares it`, async () => {
    const { calls, deps } = fixture(options);
    await assert.rejects(shareAccountExport(deps));
    assert.ok(!calls.includes("share"));
    assert.equal(calls.filter((c) => c === "close").length, 1);
    assert.equal(calls.filter((call) => call === "remove").length, 1);
    assert.ok(calls.includes("cancel"));
  });
}

test("a present Content-Length is checked in bytes without reading the file back", async () => {
  const { calls, deps } = fixture({ length: String(new TextEncoder().encode('{"format":"nyxid.account-export","note":"你好"}').length) });
  await shareAccountExport(deps);
  assert.ok(calls.includes("share"));
  assert.ok(!calls.includes("text"));
});

test("without Content-Length a complete stream tracks braces across whitespace-only chunks", async () => {
  const pieces = [" \r\n", "\t", "{", '"note":"你好"', "}", "", "\r\n \t"].map((part) => new TextEncoder().encode(part));
  const { calls, deps } = fixture({ pieces });
  await shareAccountExport(deps);
  assert.ok(calls.includes("share"));
  assert.ok(!calls.includes("text"));
  assert.equal(calls.filter((call) => call === "remove").length, 1);
});

test("without Content-Length truncated, empty and non-object streams are never shared", async () => {
  for (const body of [' \n {"note":"truncated" \t', "", "\r\n \t", "[]", '"text"', '"missing opening brace"}']) {
    const { calls, deps } = fixture({ body });
    await assert.rejects(shareAccountExport(deps), /export_incomplete/);
    assert.ok(!calls.includes("share"));
    assert.ok(!calls.includes("text"));
    assert.equal(calls.filter((call) => call === "remove").length, 1);
  }
});

type ReadResult = { done: boolean; value: Uint8Array | undefined };

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

function replaceReader(f: ReturnType<typeof fixture>, reader: {
  read(): Promise<ReadResult>;
  cancel(): Promise<void>;
  releaseLock(): void;
}) {
  const request = f.deps.request;
  f.deps.request = async (signal) => {
    const result = await request(signal);
    return { ...result, response: { ...result.response, body: { getReader: () => reader } } };
  };
}

const flush = () => new Promise<void>((resolve) => setImmediate(resolve));

function trackTimers(context: TestContext) {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const active = new Set<unknown>();
  let fired = 0;
  const schedule = globalThis.setTimeout;
  const cancel = globalThis.clearTimeout;
  context.mock.method(globalThis, "setTimeout", ((callback: () => void, delay?: number) => {
    const handle = schedule(() => {
      active.delete(handle);
      fired++;
      callback();
    }, delay);
    active.add(handle);
    return handle;
  }) as typeof setTimeout);
  context.mock.method(globalThis, "clearTimeout", (...args: Parameters<typeof clearTimeout>) => {
    active.delete(args[0]);
    return cancel(...args);
  });
  return { pending: () => active.size, fired: () => fired };
}

test("a hung native read aborts at the idle bound, removes the file and bounds cancellation", async (context) => {
  const timers = trackTimers(context);
  assert.equal(ACCOUNT_EXPORT_IDLE_TIMEOUT_MS, 30_000);
  assert.equal(ACCOUNT_EXPORT_HEADERS_TIMEOUT_MS, 75_000);
  assert.equal(ACCOUNT_EXPORT_DEADLINE_MS, 195_000);
  assert.equal(ACCOUNT_EXPORT_CANCEL_TIMEOUT_MS, 1_000);
  const f = fixture();
  const read = deferred<ReadResult>();
  replaceReader(f, {
    read: () => read.promise,
    cancel: async () => { f.calls.push("cancel"); await new Promise<never>(() => {}); },
    releaseLock: () => { f.calls.push("release"); throw new TypeError("A read is still pending"); },
  });
  const rejected = assert.rejects(shareAccountExport(f.deps), /export_idle_timeout/);
  await flush();
  const signal = f.signal();
  assert.ok(signal);
  assert.ok(f.calls.includes("create"));
  context.mock.timers.tick(ACCOUNT_EXPORT_IDLE_TIMEOUT_MS - 1);
  await flush();
  assert.equal(signal.aborted, false);
  assert.ok(!f.calls.includes("remove"));
  context.mock.timers.tick(1);
  await flush();
  assert.equal(signal.aborted, true);
  assert.equal((signal.reason as Error).message, "export_idle_timeout");
  assert.ok(f.calls.includes("remove"), "file deletion does not await native cancellation");
  assert.ok(f.calls.indexOf("remove") < f.calls.indexOf("cancel"));
  assert.equal(timers.pending(), 1, "only the bounded cancel wait remains");
  context.mock.timers.tick(ACCOUNT_EXPORT_CANCEL_TIMEOUT_MS);
  await rejected;
  assert.ok(f.calls.includes("release"), "a pending-read lock error cannot hide the timeout");
  assert.ok(!f.calls.includes("share"));
  assert.equal(timers.pending(), 0);
  assert.deepEqual(getEventListeners(signal, "abort"), []);
  read.resolve({ done: false, value: new Uint8Array([123]) });
  await flush();
  assert.ok(!f.calls.some((call) => call.startsWith("write:")), "late native bytes cannot touch a deleted file");
  context.mock.timers.tick(ACCOUNT_EXPORT_DEADLINE_MS * 2);
  assert.equal(timers.fired(), 2, "the idle and cancellation timers fired; the overall timer was cleared");
});

test("the overall deadline aborts slow progress that keeps resetting the idle watchdog", async (context) => {
  const timers = trackTimers(context);
  const f = fixture();
  let next = deferred<ReadResult>();
  replaceReader(f, {
    read: () => { next = deferred<ReadResult>(); return next.promise; },
    cancel: async () => { f.calls.push("cancel"); },
    releaseLock: () => { f.calls.push("release"); },
  });
  const rejected = assert.rejects(shareAccountExport(f.deps), /export_deadline_exceeded/);
  await flush();
  for (let i = 0; i < 9; i++) {
    context.mock.timers.tick(20_000);
    next.resolve({ done: false, value: new Uint8Array([i === 0 ? 123 : 32]) });
    await flush();
    assert.equal(f.signal()?.aborted, false);
  }
  assert.equal(f.calls.filter((call) => call.startsWith("write:")).length, 9);
  context.mock.timers.tick(15_000);
  await rejected;
  assert.equal(f.signal()?.aborted, true);
  assert.ok(f.calls.includes("remove"));
  assert.ok(!f.calls.includes("share"));
  assert.equal(timers.pending(), 0);
  assert.equal(timers.fired(), 1, "only the overall watchdog fired");
  assert.deepEqual(getEventListeners(f.signal()!, "abort"), []);
});

test("normal completion clears network watchdogs before the user-controlled share sheet", async (context) => {
  const timers = trackTimers(context);
  const f = fixture();
  const shared = deferred<void>();
  f.deps.share = async () => { f.calls.push("share"); await shared.promise; };
  const result = shareAccountExport(f.deps);
  await flush();
  assert.ok(f.calls.includes("share"));
  assert.equal(timers.pending(), 0);
  context.mock.timers.tick(ACCOUNT_EXPORT_DEADLINE_MS * 2);
  await flush();
  assert.equal(f.signal()?.aborted, false);
  shared.resolve();
  await result;
  assert.ok(f.calls.includes("remove"));
  assert.equal(timers.pending(), 0);
  assert.equal(timers.fired(), 0);
  assert.deepEqual(getEventListeners(f.signal()!, "abort"), []);
});

test("all failure exits clear their timers and remove every created file", async (context) => {
  const timers = trackTimers(context);
  for (const options of [{ available: false }, { invalid: true }, { abort: true }, { switchAccount: true }, { failShare: true }]) {
    const f = fixture(options);
    await assert.rejects(shareAccountExport(f.deps));
    assert.equal(timers.pending(), 0);
    if (f.calls.includes("create")) assert.ok(f.calls.includes("remove"));
    context.mock.timers.tick(ACCOUNT_EXPORT_DEADLINE_MS * 2);
    assert.equal(timers.fired(), 0);
    const signal = f.signal();
    if (signal) assert.deepEqual(getEventListeners(signal, "abort"), []);
  }
});

test("the header watchdog bounds a request that never returns response headers", async (context) => {
  const timers = trackTimers(context);
  const f = fixture();
  const request = f.deps.request;
  f.deps.request = async (signal) => {
    await request(signal);
    return new Promise<never>(() => {});
  };
  const rejected = assert.rejects(shareAccountExport(f.deps), /export_headers_timeout/);
  await flush();
  context.mock.timers.tick(ACCOUNT_EXPORT_IDLE_TIMEOUT_MS);
  await flush();
  assert.equal(f.signal()?.aborted, false, "body idle timer is not armed before headers");
  context.mock.timers.tick(ACCOUNT_EXPORT_HEADERS_TIMEOUT_MS - ACCOUNT_EXPORT_IDLE_TIMEOUT_MS);
  await rejected;
  assert.equal(f.signal()?.aborted, true);
  assert.ok(!f.calls.includes("create"));
  assert.equal(timers.pending(), 0);
});

test("empty chunks do not renew the no-new-bytes watchdog", async (context) => {
  const timers = trackTimers(context);
  const f = fixture();
  let next = deferred<ReadResult>();
  replaceReader(f, {
    read: () => { next = deferred<ReadResult>(); return next.promise; },
    cancel: async () => {},
    releaseLock: () => {},
  });
  const rejected = assert.rejects(shareAccountExport(f.deps), /export_idle_timeout/);
  await flush();
  for (let i = 0; i < 2; i++) {
    context.mock.timers.tick(10_000);
    next.resolve({ done: false, value: new Uint8Array() });
    await flush();
  }
  context.mock.timers.tick(10_000);
  await rejected;
  assert.equal(f.signal()?.aborted, true);
  assert.ok(f.calls.includes("remove"));
  assert.ok(!f.calls.includes("share"));
  assert.equal(timers.pending(), 0);
});

test("45-second response headers succeed without triggering the body idle watchdog", async (context) => {
  const timers = trackTimers(context);
  const f = fixture();
  const request = f.deps.request;
  const ready = deferred<void>();
  f.deps.request = async (signal) => { const result = await request(signal); await ready.promise; return result; };
  const result = shareAccountExport(f.deps);
  await flush();
  context.mock.timers.tick(45_000);
  await flush();
  assert.equal(f.signal()?.aborted, false);
  assert.ok(!f.calls.includes("create"));
  assert.equal(timers.fired(), 0);
  ready.resolve();
  await result;
  assert.ok(f.calls.includes("share"));
  assert.equal(timers.pending(), 0);
  context.mock.timers.tick(ACCOUNT_EXPORT_DEADLINE_MS * 2);
  assert.equal(timers.fired(), 0);
});

test("encoded exports use bounded structural validation instead of the wire byte count", async () => {
  for (const encoding of ["gzip", "br", " GZip "]) {
    const f = fixture({ encoding, length: "1024" });
    await shareAccountExport(f.deps);
    assert.ok(f.calls.includes("share"));
    assert.ok(!f.calls.includes("text"));
    const truncated = fixture({ encoding, length: "1024", invalid: true });
    await assert.rejects(shareAccountExport(truncated.deps), /export_incomplete/);
    assert.ok(!truncated.calls.includes("share"));
    assert.ok(truncated.calls.includes("remove"));
  }
});

test("identity encoding still requires the exact Content-Length", async () => {
  const f = fixture({ encoding: "identity", length: "1024" });
  await assert.rejects(shareAccountExport(f.deps), /export_incomplete/);
  assert.ok(!f.calls.includes("share"));
  assert.ok(f.calls.includes("remove"));
});
