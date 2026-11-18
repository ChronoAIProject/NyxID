import assert from "node:assert/strict";
import test from "node:test";
import { submitGroupDraft } from "./groupComposerDraft";
import { NO_RECIPIENT_EDITS, type RecipientSelection } from "./groupState";

const original: RecipientSelection = {
  mentioned: ["researcher"],
  added: ["writer"],
  removed: ["nyxbot"],
};

function pendingSend(draft = "  Ask @researcher  ", selection = original) {
  let value = { text: draft, selection };
  let revision = 0;
  let stale = false;
  let resolve!: (accepted: boolean) => void;
  let reject!: (cause: Error) => void;
  const admission = new Promise<boolean>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  const requests: string[] = [];
  const updates: (typeof value)[] = [];
  const updateComposer = (text: string, nextSelection: RecipientSelection) => {
    value = { text, selection: nextSelection };
    updates.push(value);
    revision++;
  };
  const submission = submitGroupDraft({
    draft,
    selection,
    updateComposer,
    getRevision: () => (stale ? NaN : revision),
    send: (text) => {
      requests.push(text);
      return admission;
    },
  });
  return {
    value: () => value,
    updateComposer,
    markStale: () => {
      stale = true;
    },
    requests,
    updates,
    resolve,
    reject,
    submission,
  };
}

test("dispatch clears text and every recipient edit while admission remains pending", async () => {
  const pending = pendingSend();
  assert.deepEqual(pending.requests, ["Ask @researcher"]);
  assert.deepEqual(pending.value(), {
    text: "",
    selection: NO_RECIPIENT_EDITS,
  });
  assert.equal(pending.updates.length, 1);
  pending.resolve(true);
  await pending.submission;
  assert.equal(pending.updates.length, 1);
});

test("accepted or uncertain completion preserves the next text and recipient choices", async () => {
  for (const text of ["A next message", ""]) {
    const pending = pendingSend();
    const nextSelection = { mentioned: [], added: ["new-agent"], removed: [] };
    pending.updateComposer(text, nextSelection);
    // Both admitted and uncertain outcomes are represented by true.
    pending.resolve(true);
    await pending.submission;
    assert.deepEqual(pending.value(), { text, selection: nextSelection });
    assert.equal(pending.updates.length, 2);
    assert.deepEqual(pending.requests, ["Ask @researcher"]);
  }
});

test("a definite refusal restores the sent text and exact snapshotted recipient edits", async () => {
  const selection = {
    mentioned: ["researcher"],
    added: ["writer"],
    removed: ["nyxbot"],
  };
  const pending = pendingSend("  Ask @researcher  ", selection);
  selection.added.push("later-mutation");
  selection.removed.length = 0;
  pending.resolve(false);
  await pending.submission;
  assert.deepEqual(pending.value(), {
    text: "Ask @researcher",
    selection: original,
  });
});

test("a newer manual recipient selection fences restoration even when text stays empty", async () => {
  const pending = pendingSend();
  const nextSelection = {
    mentioned: [],
    added: ["next-agent"],
    removed: ["writer"],
  };
  pending.updateComposer("", nextSelection);
  pending.resolve(false);
  await pending.submission;
  assert.deepEqual(pending.value(), { text: "", selection: nextSelection });
  assert.equal(pending.updates.length, 2);
});

test("an attachment-only empty draft restores its selection after refusal", async () => {
  const pending = pendingSend("");
  assert.deepEqual(pending.value(), {
    text: "",
    selection: NO_RECIPIENT_EDITS,
  });
  assert.deepEqual(pending.requests, [""]);
  pending.resolve(false);
  await pending.submission;
  assert.deepEqual(pending.value(), { text: "", selection: original });
  assert.equal(pending.updates.length, 2);
});

test("local guard errors restore the owned composer and propagate the error", async () => {
  const pending = pendingSend();
  const error = new Error("Reconnect before sending.");
  pending.reject(error);
  await assert.rejects(pending.submission, (cause) => cause === error);
  assert.deepEqual(pending.value(), {
    text: "Ask @researcher",
    selection: original,
  });
});

test("stale navigation fences refuse both text and selection restoration", async () => {
  const pending = pendingSend();
  pending.markStale();
  pending.resolve(false);
  await pending.submission;
  assert.deepEqual(pending.value(), {
    text: "",
    selection: NO_RECIPIENT_EDITS,
  });
  assert.equal(pending.updates.length, 1);
});
