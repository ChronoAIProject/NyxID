import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { deferred, hookHarness } from "./assistantHookTestHarness";
import type { useAssistantUploads } from "./useAssistantUploads";

function harness() {
  let state = "active";
  let epoch = 1;
  let focused = true;
  const listeners = new Set<(state: string) => void>();
  const picker = deferred<unknown>();
  const disposed: string[] = [];
  const requests: string[] = [];
  const props = { model: "model", generation: 0, enabled: true, online: true };
  class File {
    constructor(readonly uri: string) {}
    get exists() {
      return !disposed.includes(this.uri);
    }
    size = 3;
    delete() {
      disposed.push(this.uri);
    }
    open() {
      return { readBytes: () => new Uint8Array([1, 2, 3]), close() {} };
    }
  }
  const h = hookHarness<ReturnType<typeof useAssistantUploads>>({
    "react-native": {
      AppState: {
        get currentState() {
          return state;
        },
        addEventListener: (_: string, cb: (state: string) => void) => {
          listeners.add(cb);
          return { remove: () => listeners.delete(cb) };
        },
      },
    },
    "@react-navigation/native": { useIsFocused: () => focused },
    "expo-document-picker": { getDocumentAsync: () => picker.promise },
    "expo-file-system": { File, Paths: { cache: { uri: "file:///cache/" } } },
    "../../lib/auth/authEpoch": { getAuthEpoch: () => epoch },
    "../../lib/api/assistantUploadsApi": {
      assistantUploadsApi: {
        createDraft: async () => {
          requests.push("draft");
          return "draft-1";
        },
        upload: async () => {
          requests.push("upload");
          return {
            id: "file-1",
            label: "note.txt",
            size: 3,
            content_type: "text/plain",
            origin: "user_upload",
          };
        },
        remove: async () => {},
      },
    },
    "../../lib/api/assistantApi": {
      assistantApi: { changeModel: async () => {} },
    },
  });
  const hook = h.load(
    fileURLToPath(new URL("./useAssistantUploads.ts", import.meta.url)),
  ).useAssistantUploads as typeof useAssistantUploads;
  h.mount(() => hook(props));
  return {
    h,
    props,
    requests,
    disposed,
    emit: (next: string) => {
      state = next;
      for (const cb of listeners) cb(next);
    },
    changeOwner: () => {
      epoch++;
    },
    blur: () => {
      focused = false;
    },
    finish: () =>
      picker.resolve({
        canceled: false,
        assets: [
          {
            uri: "file:///cache/DocumentPicker/note.txt",
            name: "note.txt",
            mimeType: "text/plain",
            size: 3,
          },
        ],
      }),
  };
}

test("Android result before resume retains the file but starts no work until active effects commit", async () => {
  const x = harness();
  const picking = x.h.value().pick();
  x.emit("background");
  x.h.flush();
  x.finish();
  await picking;
  await x.h.settle();
  assert.deepEqual(x.requests, []);
  assert.deepEqual(x.disposed, []);
  assert.equal(x.h.value().files.length, 0);
  x.emit("active");
  await x.h.settle();
  assert.deepEqual(x.requests, ["draft", "upload"]);
  assert.equal(x.h.value().files[0]?.progress, 100);
  assert.equal(x.h.value().files[0]?.error, undefined);
  assert.deepEqual(x.disposed, []);
  x.h.unmount();
});

test("resume before result and a batched background/result/resume each upload exactly once", async () => {
  for (const commitBackground of [true, false]) {
    const x = harness();
    const picking = x.h.value().pick();
    x.emit("background");
    if (commitBackground) x.h.flush();
    x.emit("active");
    x.finish();
    await picking;
    await x.h.settle();
    assert.deepEqual(x.requests, ["draft", "upload"]);
    assert.equal(x.h.value().files[0]?.error, undefined);
    x.h.unmount();
  }
});

test("pending picker copies never cross owner/chat/focus/permission/connectivity or unmount boundaries", async () => {
  for (const invalidation of [
    "owner",
    "generation",
    "focus",
    "enabled",
    "online",
    "unmount",
  ] as const) {
    const x = harness();
    const picking = x.h.value().pick();
    x.emit("background");
    x.h.flush();
    x.finish();
    await picking;
    await x.h.settle();
    if (invalidation === "owner") x.changeOwner();
    else if (invalidation === "generation") x.props.generation++;
    else if (invalidation === "focus") x.blur();
    else if (invalidation === "enabled") x.props.enabled = false;
    else if (invalidation === "online") x.props.online = false;
    if (invalidation === "unmount") {
      x.h.unmount();
      await new Promise<void>((r) => setImmediate(r));
    } else {
      x.h.render();
      x.emit("active");
      await x.h.settle();
    }
    assert.deepEqual(x.requests, [], invalidation);
    assert.deepEqual(
      x.disposed,
      ["file:///cache/DocumentPicker/note.txt"],
      invalidation,
    );
    if (invalidation !== "unmount") x.h.unmount();
  }
});
