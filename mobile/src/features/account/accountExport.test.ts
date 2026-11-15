import assert from "node:assert/strict";
import test from "node:test";
import { cleanStaleAccountExports, shareAccountExport } from "./accountExport";

function fixture({ available = true, switchAccount = false, failShare = false, abort = false, length = null as string | null, invalid = false, body = undefined as string | undefined, pieces = undefined as Uint8Array[] | undefined } = {}) {
  const calls: string[] = [];
  let reads = 0;
  const text = body ?? (invalid ? '{"format":' : '{"format":"nyxid.account-export","note":"你好"}');
  const encoded = new TextEncoder().encode(text);
  const chunks = pieces ?? [encoded.slice(0, 5), encoded.slice(5)];
  return { calls, deps: {
    available: async () => { calls.push("available"); return available; },
    request: async () => { calls.push("request"); return {
      response: { headers: { get: () => length }, body: { getReader: () => ({
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
  deps.request = async () => ({ response: { headers: { get: () => null }, body: { getReader: () => ({
    read: async () => ({ done: false, value: new Uint8Array(32 * 1024 * 1024 + 1) }),
    cancel: async () => { calls.push("cancel"); }, releaseLock: () => {},
  }) } }, assertOwner: async () => {} });
  await assert.rejects(shareAccountExport(deps), /export_too_large/);
  assert.ok(!calls.some((c) => c.startsWith("write:")));
  assert.equal(calls.at(-1), "remove");
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
    assert.equal(calls.at(-1), "remove");
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
  assert.equal(calls.at(-1), "remove");
});

test("without Content-Length truncated, empty and non-object streams are never shared", async () => {
  for (const body of [' \n {"note":"truncated" \t', "", "\r\n \t", "[]", '"text"', '"missing opening brace"}']) {
    const { calls, deps } = fixture({ body });
    await assert.rejects(shareAccountExport(deps), /export_incomplete/);
    assert.ok(!calls.includes("share"));
    assert.ok(!calls.includes("text"));
    assert.equal(calls.at(-1), "remove");
  }
});
