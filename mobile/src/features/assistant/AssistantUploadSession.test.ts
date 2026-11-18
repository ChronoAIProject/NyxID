import assert from "node:assert/strict";
import test from "node:test";
import {
  AssistantUploadSession,
  type AssistantUploadedMessage,
  type PickedAssistantFile,
} from "./AssistantUploadSession";
import {
  ATTACHMENT_SIZE_MESSAGE,
  MAX_ASSISTANT_ATTACHMENT_BYTES,
  UNSUPPORTED_ATTACHMENT_MESSAGE,
} from "./assistantUploads";
import { submitAssistantDraft } from "./assistantComposerDraft";
import type { AssistantAttachment } from "./assistantTypes";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (cause: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

async function until(check: () => boolean) {
  for (let step = 0; step < 100; step++) {
    if (check()) return;
    await new Promise<void>((resolve) => setImmediate(resolve));
  }
  assert.ok(check(), "expected session transition did not occur");
}

const file = (name = "notes.txt", size?: number): PickedAssistantFile => ({
  uri: `file:///cache/${name}`,
  name,
  mimeType: "text/plain",
  size,
});
const accepted = (name = "notes.txt"): AssistantAttachment => ({
  id: `attachment-${name}`,
  content_type: "text/plain",
  label: name,
  size: 3,
  origin: "user_upload",
});
type Dependencies = ConstructorParameters<typeof AssistantUploadSession>[0];

function harness(
  overrides: Partial<Dependencies> = {},
  conversationId?: string,
) {
  let current = true;
  let available = true;
  const drafts: AbortSignal[] = [];
  const reads: string[] = [];
  const stats: string[] = [];
  const uploads: { id: string; name: string; signal: AbortSignal }[] = [];
  const removals: { id: string; attachmentId: string }[] = [];
  const disposed: string[] = [];
  let notifications = 0;
  const session = new AssistantUploadSession(
    {
      current: () => current,
      available: () => available,
      changed: () => {
        notifications++;
      },
      fileSize: (item) => {
        stats.push(item.name);
        return 3;
      },
      readBytes: async (item) => {
        reads.push(item.name);
        return new Uint8Array([1, 2, 3]);
      },
      createDraft: async (signal) => {
        drafts.push(signal);
        return "nyxa-upload-draft";
      },
      upload: async (id, item, _bytes, signal) => {
        uploads.push({ id, name: item.name, signal });
        return accepted(item.name);
      },
      remove: async (id, attachmentId) => {
        removals.push({ id, attachmentId });
      },
      disposeFile: (item) => {
        disposed.push(item.name);
      },
      ...overrides,
    },
    conversationId,
  );
  return {
    session,
    drafts,
    stats,
    reads,
    uploads,
    removals,
    disposed,
    setCurrent: (value: boolean) => {
      current = value;
    },
    setAvailable: (value: boolean) => {
      available = value;
    },
    notifications: () => notifications,
  };
}

test("multiple selections upload serially into one durable draft", async () => {
  const first = deferred<AssistantAttachment>();
  const started: string[] = [];
  const h = harness({
    upload: async (id, item) => {
      assert.equal(id, "nyxa-upload-draft");
      started.push(item.name);
      return item.name === "one.txt" ? first.promise : accepted(item.name);
    },
  });
  h.session.add([file("one.txt"), file("two.txt")]);
  h.session.add([file("three.txt")]);
  await until(() => started.length === 1);
  assert.deepEqual(started, ["one.txt"]);
  assert.deepEqual(h.reads, ["one.txt"]);
  assert.equal(h.drafts.length, 1);
  assert.equal(h.session.blocked, true);
  first.resolve(accepted("one.txt"));
  await until(() => !h.session.blocked);
  assert.deepEqual(started, ["one.txt", "two.txt", "three.txt"]);
  assert.equal(h.drafts.length, 1);
  h.session.dispose();
});

test("dictation and attachments share the same in-flight durable draft", async () => {
  const draft = deferred<string>();
  let creates = 0;
  const h = harness({ createDraft: () => { creates++; return draft.promise; } });
  const dictation = h.session.ensureConversation();
  h.session.add([file()]);
  await until(() => creates === 1 && h.reads.length === 1);
  draft.resolve("nyxa-shared-draft");
  assert.equal(await dictation, "nyxa-shared-draft");
  await until(() => !h.session.blocked);
  assert.equal(creates, 1);
  assert.equal(h.uploads[0]?.id, "nyxa-shared-draft");
  assert.equal(await h.session.ensureConversation(), "nyxa-shared-draft");
  h.session.dispose();
});

test("unknown or stale picker size cannot trigger a giant native file read", async () => {
  for (const pickerSize of [undefined, 3]) {
    const h = harness({ fileSize: () => MAX_ASSISTANT_ATTACHMENT_BYTES + 1 });
    h.session.add([file("huge.txt", pickerSize)]);
    await until(() => !!h.session.entries.get("1")?.error);
    assert.equal(h.session.entries.get("1")?.error, ATTACHMENT_SIZE_MESSAGE);
    assert.deepEqual(h.reads, []);
    assert.equal(h.drafts.length, 0);
    assert.equal(h.uploads.length, 0);
    h.session.dispose();
  }
});

test("invalid picker format and zero size fail before native stat or upload", () => {
  const h = harness();
  h.session.add([
    { ...file("archive.zip", 3), mimeType: "application/zip" },
    file("empty.txt", 0),
  ]);
  assert.equal(
    h.session.entries.get("1")?.error,
    UNSUPPORTED_ATTACHMENT_MESSAGE,
  );
  assert.equal(h.session.entries.get("2")?.error, ATTACHMENT_SIZE_MESSAGE);
  assert.equal(h.session.blocked, true);
  assert.deepEqual(h.stats, []);
  h.session.dispose();
});

test("an empty or changed read never creates a draft or uploads bytes", async () => {
  const h = harness({ readBytes: async () => new Uint8Array() });
  h.session.add([file()]);
  await until(() => !!h.session.entries.get("1")?.error);
  assert.equal(h.session.entries.get("1")?.error, ATTACHMENT_SIZE_MESSAGE);
  assert.equal(h.drafts.length, 0);
  assert.equal(h.uploads.length, 0);
  h.session.dispose();
});

test("failed rows use attachment capacity and block send until removed", async () => {
  const h = harness();
  h.session.add(
    Array.from({ length: 12 }, (_, i) => ({
      ...file(`bad-${i}.zip`),
      mimeType: "application/zip",
    })),
  );
  assert.equal(h.session.entries.size, 10);
  assert.deepEqual(h.disposed, ["bad-10.zip", "bad-11.zip"]);
  assert.match(h.session.notice ?? "", /Up to 10 files/);
  let sends = 0;
  await assert.rejects(
    h.session.submit(async () => {
      sends++;
      return true;
    }),
    /remove failed files/,
  );
  assert.equal(sends, 0);
  for (const key of h.session.entries.keys()) h.session.remove(key);
  assert.equal(h.session.blocked, false);
  assert.equal(h.session.entries.size, 0);
  h.session.dispose();
});

test("an upload error does not poison the serial queue for later selections", async () => {
  const h = harness({
    upload: async (_id, item) => {
      if (item.name === "bad.txt")
        throw new Error("Malformed or unsupported attachment");
      return accepted(item.name);
    },
  });
  h.session.add([file("bad.txt"), file("good.txt")]);
  await until(() => !!h.session.entries.get("2")?.item);
  assert.equal(
    h.session.entries.get("1")?.error,
    UNSUPPORTED_ATTACHMENT_MESSAGE,
  );
  assert.equal(h.session.blocked, true);
  h.session.remove("1");
  assert.equal(h.session.blocked, false);
  assert.equal(h.drafts.length, 1);
  h.session.dispose();
});

test("a failed draft creation can retry without replacing an accepted draft later", async () => {
  let requests = 0;
  const h = harness({
    createDraft: async () => {
      if (++requests === 1) throw new Error("Draft creation refused");
      return "nyxa-retried-draft";
    },
  });
  h.session.add([file("one.txt"), file("two.txt"), file("three.txt")]);
  await until(() => !!h.session.entries.get("3")?.item);
  assert.equal(requests, 2);
  assert.equal(h.session.entries.get("1")?.error, "Draft creation refused");
  assert.deepEqual(
    h.uploads.map((row) => row.id),
    ["nyxa-retried-draft", "nyxa-retried-draft"],
  );
  h.session.dispose();
});

test("availability guards reject picker additions and submit while inactive", async () => {
  const h = harness();
  h.setAvailable(false);
  h.session.add([file()]);
  assert.equal(h.session.entries.size, 0);
  assert.deepEqual(h.disposed, ["notes.txt"]);
  await assert.rejects(
    h.session.submit(async () => true),
    /keep this chat open/,
  );
  assert.equal(h.drafts.length, 0);
  h.session.dispose();
});

test("going inactive while stat or read is pending prevents further native/network work", async () => {
  const stat = deferred<number>();
  const h = harness({ fileSize: () => stat.promise });
  h.session.add([file()]);
  await new Promise<void>((resolve) => setImmediate(resolve));
  h.setAvailable(false);
  stat.resolve(3);
  await until(() => !!h.session.entries.get("1")?.error);
  assert.deepEqual(h.reads, []);
  assert.equal(h.drafts.length, 0);
  h.session.dispose();

  const bytes = deferred<Uint8Array>();
  const second = harness({ readBytes: () => bytes.promise });
  second.session.add([file()]);
  await new Promise<void>((resolve) => setImmediate(resolve));
  second.setAvailable(false);
  bytes.resolve(new Uint8Array([1, 2, 3]));
  await until(() => !!second.session.entries.get("1")?.error);
  assert.equal(second.drafts.length, 0);
  second.session.dispose();
});

test("removing a file during draft creation prevents its upload and reuses that draft", async () => {
  const draft = deferred<string>();
  let requests = 0;
  const h = harness({
    createDraft: () => {
      requests++;
      return draft.promise;
    },
  });
  h.session.add([file("one.txt"), file("two.txt")]);
  await until(() => requests === 1);
  h.session.remove("1");
  draft.resolve("nyxa-still-useful-draft");
  await until(() => !!h.session.entries.get("2")?.item);
  assert.equal(requests, 1);
  assert.deepEqual(
    h.uploads.map((row) => row.name),
    ["two.txt"],
  );
  assert.equal(h.uploads[0]?.id, "nyxa-still-useful-draft");
  h.session.dispose();
});

test("late upload acceptance after removal deletes only that scoped upload", async () => {
  const upload = deferred<AssistantAttachment>();
  let started = false;
  let signal: AbortSignal | undefined;
  const h = harness({
    upload: (_id, _file, _bytes, controller) => {
      started = true;
      signal = controller;
      return upload.promise;
    },
  });
  h.session.add([file()]);
  await until(() => started);
  h.session.remove("1");
  assert.equal(signal?.aborted, true);
  upload.resolve(accepted());
  await until(() => h.removals.length === 1);
  assert.deepEqual(h.removals, [
    { id: "nyxa-upload-draft", attachmentId: "attachment-notes.txt" },
  ]);
  assert.equal(h.session.entries.size, 0);
  assert.deepEqual(h.disposed, ["notes.txt"]);
  h.session.dispose();
});

test("interrupt preserves ready files and aborts queued or uploading files", async () => {
  const upload = deferred<AssistantAttachment>();
  let secondStarted = false;
  const h = harness({
    upload: async (_id, item) => {
      if (item.name === "two.txt") {
        secondStarted = true;
        return upload.promise;
      }
      return accepted(item.name);
    },
  });
  h.session.add([file("one.txt"), file("two.txt"), file("three.txt")]);
  await until(() => secondStarted);
  h.setAvailable(false);
  h.session.interrupt();
  assert.ok(h.session.entries.get("1")?.item);
  assert.equal(h.session.entries.get("2")?.controller.signal.aborted, true);
  assert.equal(h.session.entries.get("3")?.controller.signal.aborted, true);
  assert.match(h.session.entries.get("2")?.error ?? "", /Upload interrupted/);
  h.setAvailable(true);
  upload.resolve(accepted("two.txt"));
  await until(() => h.removals.length === 1);
  assert.equal(h.session.entries.get("2")?.item, undefined);
  assert.deepEqual(h.reads, ["one.txt", "two.txt"]);
  assert.equal(h.session.blocked, true);
  h.session.dispose();
});

test("late acceptance from an old generation cannot alter the new session", async () => {
  const upload = deferred<AssistantAttachment>();
  let started = false;
  const old = harness({
    upload: () => {
      started = true;
      return upload.promise;
    },
  });
  old.session.add([file()]);
  await until(() => started);
  old.setCurrent(false);
  old.session.dispose();
  old.session.dispose();
  const notifications = old.notifications();
  const next = harness({}, "nyxa-new-chat");
  next.session.add([file("next.txt")]);
  await until(() => !next.session.blocked);
  upload.resolve(accepted());
  await until(() => old.removals.length === 1);
  assert.equal(old.notifications(), notifications);
  assert.equal(old.session.entries.size, 0);
  assert.deepEqual(old.disposed, ["notes.txt"]);
  assert.equal(next.session.entries.get("1")?.item?.label, "next.txt");
  next.session.dispose();
});

test("a late draft response after disposal cannot start an upload", async () => {
  const draft = deferred<string>();
  let started = false;
  let signal: AbortSignal | undefined;
  const h = harness({
    createDraft: (controller) => {
      started = true;
      signal = controller;
      return draft.promise;
    },
  });
  h.session.add([file()]);
  await until(() => started);
  h.session.dispose();
  assert.equal(signal?.aborted, true);
  draft.resolve("nyxa-old-draft");
  await new Promise<void>((resolve) => setImmediate(resolve));
  assert.equal(h.uploads.length, 0);
  assert.equal(h.session.entries.size, 0);
});

test("progress stays below Ready until the server accepts the upload", async () => {
  const upload = deferred<AssistantAttachment>();
  let progress: ((value: number) => void) | undefined;
  const h = harness({
    upload: (_id, _file, _bytes, _signal, update) => {
      progress = update;
      return upload.promise;
    },
  });
  h.session.add([file()]);
  await until(() => !!progress);
  progress!(100);
  assert.equal(h.session.entries.get("1")?.progress, 99);
  assert.equal(h.session.blocked, true);
  upload.resolve(accepted());
  await until(() => !h.session.blocked);
  assert.equal(h.session.entries.get("1")?.progress, 100);
  h.session.dispose();
});

test("refused send restores both text and attachment IDs for the same draft", async () => {
  const h = harness();
  h.session.add([file()]);
  await until(() => !h.session.blocked);
  let draft = "  message  ";
  let revision = 0;
  let payload: AssistantUploadedMessage | undefined;
  await submitAssistantDraft({
    draft,
    updateDraft: (text) => {
      draft = text;
      revision++;
    },
    getRevision: () => revision,
    send: (text) =>
      h.session.submit(async (uploads) => {
        assert.equal(text, "message");
        assert.equal(draft, "");
        assert.equal(h.session.entries.size, 0);
        payload = uploads;
        return false;
      }),
  });
  assert.equal(draft, "message");
  assert.equal(payload?.conversationId, "nyxa-upload-draft");
  assert.deepEqual(payload?.attachmentIds, ["attachment-notes.txt"]);
  assert.equal(h.session.entries.get("1")?.item?.id, "attachment-notes.txt");
  assert.equal(h.removals.length, 0);
  assert.equal(h.disposed.length, 0);
  h.session.dispose();
});

test("accepted or uncertain admission consumes attachments and never deletes bound bytes", async () => {
  const stream = deferred<boolean>();
  const h = harness();
  h.session.add([file()]);
  await until(() => !h.session.blocked);
  let dispatched = false;
  const submission = h.session.submit(async () => {
    dispatched = true;
    return stream.promise;
  });
  await until(() => dispatched);
  assert.equal(h.session.entries.size, 0);
  assert.equal(h.session.sending, true);
  await assert.rejects(
    h.session.submit(async () => true),
    /already being sent/,
  );
  h.session.dispose();
  assert.deepEqual(h.disposed, ["notes.txt"]);
  assert.equal(h.removals.length, 0);
  stream.resolve(true);
  assert.equal(await submission, true);
  assert.equal(h.session.entries.size, 0);
  assert.equal(h.removals.length, 0);
  assert.deepEqual(h.disposed, ["notes.txt"]);
});

test("a local send guard restores attachments and propagates its error", async () => {
  const h = harness();
  h.session.add([file()]);
  await until(() => !h.session.blocked);
  await assert.rejects(
    h.session.submit(async () => {
      throw new Error("Saved chat not loaded");
    }),
    /Saved chat not loaded/,
  );
  assert.ok(h.session.entries.get("1")?.item);
  assert.equal(h.session.sending, false);
  assert.equal(h.removals.length, 0);
  h.session.dispose();
});

test("a late refusal after navigating away cleans files without restoring another chat", async () => {
  const request = deferred<boolean>();
  const h = harness();
  h.session.add([file()]);
  await until(() => !h.session.blocked);
  let dispatched = false;
  const submission = h.session.submit(async () => {
    dispatched = true;
    return request.promise;
  });
  await until(() => dispatched);
  h.setCurrent(false);
  h.session.dispose();
  request.resolve(false);
  assert.equal(await submission, false);
  assert.equal(h.session.entries.size, 0);
  assert.deepEqual(h.removals, [
    { id: "nyxa-upload-draft", attachmentId: "attachment-notes.txt" },
  ]);
  assert.deepEqual(h.disposed, ["notes.txt"]);
});

test("saved conversations upload into their existing thread without creating drafts", async () => {
  const h = harness({}, "nyxa-saved-chat");
  h.session.add([file()]);
  await until(() => !h.session.blocked);
  assert.equal(
    await h.session.submit(async (uploads) => {
      assert.equal(uploads?.conversationId, "nyxa-saved-chat");
      return true;
    }),
    true,
  );
  assert.equal(h.drafts.length, 0);
  h.session.dispose();
});

test("plain text submission retains the send hook's boolean result", async () => {
  const h = harness();
  assert.equal(
    await h.session.submit(async (uploads) => {
      assert.equal(uploads, undefined);
      return false;
    }),
    false,
  );
  assert.equal(
    await h.session.submit(async (uploads) => {
      assert.equal(uploads, undefined);
      return true;
    }),
    true,
  );
  assert.equal(h.drafts.length, 0);
  h.session.dispose();
});

test("attachment handles count independently by kind and never change when earlier rows are removed", async () => {
  const h = harness();
  h.session.add([
    file("first.txt"),
    { ...file("chart.png"), mimeType: "image/png" },
    file("second.txt"),
  ]);
  await until(() => !h.session.blocked);
  assert.deepEqual(
    [...h.session.entries.values()].map(({ key, handle, kind }) => ({
      key,
      handle,
      kind,
    })),
    [
      { key: "1", handle: "file-1", kind: "file" },
      { key: "2", handle: "image-1", kind: "image" },
      { key: "3", handle: "file-2", kind: "file" },
    ],
  );
  const second = h.session.entries.get("3");
  h.session.remove("1");
  h.session.add([
    file("third.txt"),
    { ...file("next.jpg"), mimeType: "image/jpeg" },
  ]);
  await until(() => !h.session.blocked);
  assert.equal(h.session.entries.get("3"), second);
  assert.deepEqual(
    [...h.session.entries.values()].map(({ handle }) => handle),
    ["image-1", "file-2", "file-3", "image-2"],
  );
  h.session.dispose();
});

test("refused sends restore exactly the same handles while accepted sends never reuse them", async () => {
  const h = harness();
  h.session.add([file("first.txt")]);
  await until(() => !h.session.blocked);
  const first = h.session.entries.get("1");
  assert.equal(first?.handle, "file-1");
  assert.equal(await h.session.submit(async () => false), false);
  assert.equal(h.session.entries.get("1"), first);
  assert.equal(h.session.entries.get("1")?.handle, "file-1");
  assert.equal(await h.session.submit(async () => true), true);
  h.session.add([file("next.txt")]);
  await until(() => !h.session.blocked);
  assert.equal(h.session.entries.get("2")?.handle, "file-2");
  h.session.dispose();
});

test("validation and upload failures retain their assigned reference and consume its number", async () => {
  const h = harness({
    upload: async (_id, item) => {
      if (item.name === "failed.txt") throw new Error("Upload refused");
      return accepted(item.name);
    },
  });
  h.session.add([
    { ...file("invalid.zip"), mimeType: "application/zip" },
    file("failed.txt"),
  ]);
  const failed = h.session.entries.get("2");
  assert.equal(failed?.handle, "file-2");
  await until(() => !!failed?.error);
  assert.equal(h.session.entries.get("1")?.handle, "file-1");
  assert.equal(failed?.handle, "file-2");
  assert.equal(failed?.error, "Upload refused");
  h.session.remove("1");
  h.session.remove("2");
  h.session.add([file("success.txt")]);
  await until(() => !h.session.blocked);
  assert.equal(h.session.entries.get("3")?.handle, "file-3");
  h.session.dispose();
});

test("attachment allocation skips current group handles case-insensitively and reads membership live", async () => {
  let members: readonly string[] = ["FILE-1", "file-3", "IMAGE-1"];
  const h = harness({ reservedMentionHandles: () => members }, "group-id");
  h.session.add([
    file("first.txt"),
    { ...file("chart.png"), mimeType: "image/png" },
  ]);
  await until(() => !h.session.blocked);
  assert.equal(h.session.entries.get("1")?.handle, "file-2");
  assert.equal(h.session.entries.get("2")?.handle, "image-2");
  members = ["file-4", "file-5"];
  h.session.add([file("second.txt")]);
  await until(() => !h.session.blocked);
  assert.equal(h.session.entries.get("3")?.handle, "file-3");
  h.session.add([file("third.txt")]);
  await until(() => !h.session.blocked);
  assert.equal(h.session.entries.get("4")?.handle, "file-6");
  assert.equal(h.session.entries.get("1")?.handle, "file-2");
  h.session.dispose();
});
