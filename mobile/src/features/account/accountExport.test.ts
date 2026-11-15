import assert from "node:assert/strict";
import test from "node:test";
import { cleanStaleAccountExports, shareAccountExport } from "./accountExport";

function fixture({ available = true, switchAccount = false, failShare = false } = {}) {
  const calls: string[] = [];
  let reads = 0;
  const chunks = [new Uint8Array([123,34]), new Uint8Array([120,34,125])];
  return { calls, deps: {
    available: async () => { calls.push("available"); return available; },
    request: async () => { calls.push("request"); return {
      response: { body: { getReader: () => ({
        read: async () => { const value = chunks[reads++]; return { done: !value, value }; },
        cancel: async () => { calls.push("cancel"); }, releaseLock: () => { calls.push("release"); },
      }) } },
      assertOwner: async () => { calls.push("owner"); if (switchAccount && reads > 0) throw new Error("auth_session_changed"); },
    }; },
    create: () => { calls.push("create"); return {
      uri: "cache/export.json", write: (bytes: Uint8Array) => { calls.push(`write:${bytes.length}`); },
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
  assert.deepEqual(calls.filter((c) => c.startsWith("write:")), ["write:2", "write:3"]);
  assert.ok(calls.indexOf("close") < calls.indexOf("share"));
  assert.equal(calls.at(-1), "remove");
  assert.equal(calls.filter((c) => c === "owner").length, 5);
});

test("account switches and share failures close and remove the file without claiming success", async () => {
  for (const options of [{ switchAccount: true }, { failShare: true }]) {
    const { calls, deps } = fixture(options);
    await assert.rejects(shareAccountExport(deps));
    assert.equal(calls.at(-1), "remove");
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
  deps.request = async () => ({ response: { body: { getReader: () => ({
    read: async () => ({ done: false, value: new Uint8Array(32 * 1024 * 1024 + 1) }),
    cancel: async () => { calls.push("cancel"); }, releaseLock: () => {},
  }) } }, assertOwner: async () => {} });
  await assert.rejects(shareAccountExport(deps), /export_too_large/);
  assert.ok(!calls.some((c) => c.startsWith("write:")));
  assert.equal(calls.at(-1), "remove");
});
