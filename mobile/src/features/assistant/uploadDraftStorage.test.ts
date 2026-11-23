import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { buildApiError } from "../../lib/api/responseError";
import { AssistantUploadSession } from "./AssistantUploadSession";
import { deferred, hookHarness } from "./assistantHookTestHarness";

const key = JSON.stringify(["private", "nyxbot", "first"]);
const item = { id: "pending-id", label: "one.txt", size: 3, content_type: "text/plain", origin: "user_upload", expired: false };
const draft = {
  conversationId: "nyxa-original",
  rows: [
    { handle: "file-1", kind: "file" as const, file: { name: "one.txt", path: "DocumentPicker/one.txt" }, item },
    { handle: "file-2", kind: "file" as const, file: { name: "outside.txt", path: "%2e%2e/outside.txt" } },
  ],
  reselect: false,
};

function setup(cacheRoot = "file:///cache/") {
  let account: string | null = "A";
  let epoch = 1;
  const accountListeners = new Set<() => void>();
  const disk = new Map<string, string>();
  const files: string[] = [];
  const calls: { path: string; owner: string; signal: AbortSignal }[] = [];
  const timer = new Set<() => void>();
  let remote: () => Promise<unknown> = async () => {};
  const h = hookHarness({
    "@react-native-async-storage/async-storage": { __esModule: true, default: {
      getItem: async (key: string) => disk.get(key) ?? null,
      setItem: async (key: string, value: string) => { disk.set(key, value); },
      removeItem: async (key: string) => { disk.delete(key); },
      getAllKeys: async () => [...disk.keys()],
      multiRemove: async (keys: string[]) => { keys.forEach((key) => disk.delete(key)); },
    } },
    "../../lib/auth/authAccount": {
      getAuthAccountId: () => account,
      subscribeAuthAccount: (listener: () => void) => {
        accountListeners.add(listener);
        return () => accountListeners.delete(listener);
      },
    },
    "../../lib/auth/authEpoch": { getAuthEpoch: () => epoch },
    "expo-file-system": {
      Paths: { cache: { uri: cacheRoot } },
      File: class {
        constructor(readonly uri: string) {}
        get exists() { return !files.includes(this.uri); }
        delete() { files.push(this.uri); }
      },
    },
    "./http": {
      requestJson: async (path: string, options: any) => {
        options.authorityGuard();
        assert.equal(options.expectedAccountId, account);
        assert.equal(options.method, "DELETE");
        calls.push({ path, owner: options.expectedAccountId, signal: options.signal });
        return remote();
      },
    },
  }, {
    setTimeout: (cb: () => void) => { timer.add(cb); return cb; },
    clearTimeout: (cb: () => void) => timer.delete(cb),
  });
  // Run the actual timer, API classifier, AsyncStorage adapter and draft store.
  const operations = h.load(fileURLToPath(new URL("./assistantUploads.ts", import.meta.url)));
  h.mock("./assistantUploads", operations);
  h.mock("../../features/assistant/assistantUploads", operations);
  const api = h.load(
    fileURLToPath(new URL("../../lib/api/assistantUploadsApi.ts", import.meta.url)),
  ) as typeof import("../../lib/api/assistantUploadsApi");
  h.mock("../../lib/api/assistantUploadsApi", api);
  const storage = h.load(
    fileURLToPath(new URL("./uploadDraftStorage.ts", import.meta.url)),
  ) as typeof import("./uploadDraftStorage");
  const { discardStoredUploadDraft, uploadDraftStorage } = storage;
  h.mock("./uploadDraftStorage", storage);
  const store = h.load(fileURLToPath(new URL("./assistantDraftStore.ts", import.meta.url))) as typeof import("./assistantDraftStore");
  return {
    files, calls, uploadDraftStorage, discardStoredUploadDraft, ...api, ...store,
    expire: () => Array.from(timer).forEach((callback) => callback()),
    setRemote: (handler: typeof remote) => { remote = handler; },
    switchTo: (owner: string) => {
      account = owner;
      epoch++;
      for (const notify of accountListeners) notify();
    },
  };
}

for (const root of ["file:///cache", "file:///cache/"]) {
  test(`cold discard releases rooted local and owner-scoped pending remote files from ${root}`, async () => {
    const x = setup(root);
    await x.uploadDraftStorage.save("A", key, draft);
    await x.uploadDraftStorage.save("B", key, draft);
    x.setRemote(async () => {
      throw buildApiError({
        error: "not_found", error_code: 1003, message: "Not found: Attachment not found or expired",
      }, 404);
    });
    await x.discardStoredUploadDraft("A", key, 1);
    assert.deepEqual(x.files, ["file:///cache/DocumentPicker/one.txt"]);
    assert.equal(x.calls[0]?.path, "/assistant/nyxagent/conversations/nyxa-original/attachments/pending-id");
    assert.equal(x.calls[0]?.owner, "A");
    assert.equal(await x.uploadDraftStorage.load("A", key), undefined);
    assert.ok(await x.uploadDraftStorage.load("B", key));
  });
}

test("offline discard keeps a blocked cleanup record and prevents a stale save from reviving it", async () => {
  const x = setup();
  await x.uploadDraftStorage.save("A", key, { ...draft, model: "nyxagent/research" });
  x.setRemote(async () => { throw buildApiError(null, 0); });
  const discard = x.discardStoredUploadDraft("A", key, 1);
  const load = x.uploadDraftStorage.load("A", key);
  const staleSave = x.uploadDraftStorage.save("A", key, draft);
  await assert.rejects(discard);
  await assert.rejects(staleSave, /pending discard/);
  const stored = await load;
  assert.equal(stored?.discardPending, true);
  assert.equal(stored?.model, undefined);
  assert.equal(stored?.rows[0]?.item?.id, item.id);
  x.setRemote(async () => {
    throw buildApiError({
      error: "not_found", error_code: 1003, message: "Not found: Conversation not found",
    }, 404);
  });
  await x.discardStoredUploadDraft("A", key, 1);
  assert.equal(await x.uploadDraftStorage.load("A", key), undefined);
  assert.equal(x.files.length, 1);
});

test("account switch aborts a hung cleanup, clears ordinary drafts and retains only discard retry data", async () => {
  const x = setup();
  const started = deferred<void>();
  const response = deferred<void>();
  x.setRemote(() => { started.resolve(); return response.promise; });
  const ordinaryKey = JSON.stringify(["private", "nyxbot", "unsubmitted"]);
  await x.uploadDraftStorage.save("A", key, { ...draft, model: "nyxagent/research" });
  await x.uploadDraftStorage.save("A", ordinaryKey, {
    ...draft, rows: [{ ...draft.rows[0]!, file: { name: "ordinary.txt", path: "ordinary.txt" } }],
  });
  await x.uploadDraftStorage.save("B", key, draft);
  const discard = x.discardStoredUploadDraft("A", key, 1);
  await started.promise;
  x.switchTo("B");
  await assert.rejects(discard);
  assert.equal(x.calls[0]?.signal.aborted, true);
  assert.equal(await x.uploadDraftStorage.load("A", ordinaryKey), undefined);
  assert.ok(x.files.includes("file:///cache/ordinary.txt"));
  assert.equal((await x.uploadDraftStorage.load("A", key))?.discardPending, true);
  assert.equal((await x.uploadDraftStorage.load("A", key))?.model, undefined);
  assert.equal((await x.uploadDraftStorage.load("B", key))?.discardPending, undefined);
  response.resolve();
  assert.equal(x.calls.length, 1);
});

test("a hung live DELETE times out, preserves the receipt, and permits explicit discard retry", async () => {
  const x = setup();
  let started = deferred<void>();
  const response = deferred<void>();
  x.setRemote(() => { started.resolve(); return response.promise; });
  const session = new AssistantUploadSession({
    current: () => true,
    available: () => true,
    changed: () => {},
    createDraft: async () => "nyxa-original",
    fileSize: () => 3,
    readBytes: async () => new Uint8Array(3),
    upload: async () => item,
    remove: (id, attachment, signal) => x.assistantUploadsApi.remove(
      { kind: "conversations", id }, attachment, { signal, expectedAccountId: "A", authorityGuard: () => {} },
    ),
    disposeFile: () => {},
    saveDraft: (value) => x.uploadDraftStorage.save("A", key, value.snapshot((uri) => uri.replace("file:///cache/", ""))),
  });
  session.restore({ ...draft, rows: [draft.rows[0]!] }, (path) => "file:///cache/" + path);
  await session.saveDraft();
  x.assistantDraftStore.set(`uploads:${key}`, session);
  session.remove("1");
  await started.promise;
  x.expire();
  for (let i = 0; i < 8; i++) await Promise.resolve();
  assert.equal(x.calls[0]?.signal.aborted, true);
  assert.equal(session.entries.get("1")?.removing, false);
  assert.equal(session.entries.get("1")?.item?.id, item.id);
  assert.match(session.notice ?? "", /could not be removed/);

  // A second in-flight removal must also yield to the foreground discard.
  started = deferred<void>();
  session.remove("1");
  await started.promise;
  x.setRemote(async () => {});
  await x.discardUploadDraft("A", key);
  assert.equal(x.calls[1]?.signal.aborted, true);
  assert.equal(x.calls.length, 3);
  assert.equal(await x.uploadDraftStorage.load("A", key), undefined);
  assert.equal(session.disposed, true);
  response.resolve();
});

test("preparing discard serializes only the attachment cleanup state", async () => {
  const x = setup();
  const session = new AssistantUploadSession({
    current: () => true,
    available: () => true,
    changed: () => {},
    createDraft: async () => "nyxa-original",
    fileSize: () => 3,
    readBytes: async () => new Uint8Array(3),
    upload: async () => item,
    remove: async () => {},
    disposeFile: () => {},
    saveDraft: (value) => x.uploadDraftStorage.save("A", key, value.snapshot((uri) => uri.replace("file:///cache/", ""))),
  });
  session.restore({ ...draft, model: "nyxagent/research", keepConversation: true }, (path) => "file:///cache/" + path);
  await session.prepareDiscard();
  const saved = await x.uploadDraftStorage.load("A", key);
  assert.equal(saved?.discardPending, true);
  assert.equal(saved?.model, undefined);
  assert.equal(saved?.keepConversation, false);
  assert.equal(saved?.conversationId, "nyxa-original");
  assert.equal(saved?.rows[0]?.item?.id, item.id);
});
