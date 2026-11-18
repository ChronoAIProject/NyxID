import assert from "node:assert/strict";
import test from "node:test";
import { submitAssistantDraft } from "./assistantComposerDraft";

function pendingSend(draft = "  message to send  ") {
  let value = draft;
  let revision = 0;
  let resolve!: (accepted: boolean) => void;
  let reject!: (cause: Error) => void;
  const stream = new Promise<boolean>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  const requests: string[] = [];
  const updateDraft = (text: string) => {
    value = text;
    revision++;
  };
  const submission = submitAssistantDraft({
    draft,
    updateDraft,
    getRevision: () => revision,
    send: (text) => {
      requests.push(text);
      return stream;
    },
  });
  return {
    value: () => value,
    updateDraft,
    requests,
    resolve,
    reject,
    submission,
  };
}

test("the native field clears while the reply stream is still pending", async () => {
  const pending = pendingSend();
  assert.deepEqual(pending.requests, ["message to send"]);
  assert.equal(pending.value(), "");
  pending.resolve(true);
  await pending.submission;
  assert.equal(pending.value(), "");
});

test("a definite pre-admission refusal restores the editable sent message", async () => {
  const pending = pendingSend();
  pending.resolve(false);
  await pending.submission;
  assert.equal(pending.value(), "message to send");
});

test("a local pre-dispatch guard restores text and still reports its error", async () => {
  const pending = pendingSend();
  const error = new Error("Reconnect to send a message.");
  pending.reject(error);
  await assert.rejects(pending.submission, (cause) => cause === error);
  assert.equal(pending.value(), "message to send");
});

test("reply completion never clears text typed for the next message", async () => {
  const pending = pendingSend();
  pending.updateDraft("next message");
  pending.resolve(true);
  await pending.submission;
  assert.equal(pending.value(), "next message");
});

test("a refused send cannot replace a newer draft", async () => {
  const pending = pendingSend();
  pending.updateDraft("newer draft");
  pending.resolve(false);
  await pending.submission;
  assert.equal(pending.value(), "newer draft");
});

test("editing then clearing or switching chats blocks an old refusal's restore", async () => {
  for (const newerText of ["draft later cleared", ""]) {
    const pending = pendingSend();
    pending.updateDraft(newerText);
    pending.updateDraft("");
    pending.resolve(false);
    await pending.submission;
    assert.equal(pending.value(), "");
  }
});

test("an old local error cannot replace text after navigation", async () => {
  const pending = pendingSend();
  pending.updateDraft("");
  pending.updateDraft("draft in another chat");
  pending.reject(new Error("Wait for this chat's saved history to load."));
  await assert.rejects(pending.submission, /saved history/);
  assert.equal(pending.value(), "draft in another chat");
});

test("an accepted or uncertain send stays cleared and is never replayed", async () => {
  const pending = pendingSend();
  // The hook returns true for admitted sends even if the reply is interrupted.
  pending.resolve(true);
  await pending.submission;
  assert.equal(pending.value(), "");
  assert.deepEqual(pending.requests, ["message to send"]);
});
