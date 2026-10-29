import assert from "node:assert/strict";
import test from "node:test";
import {
  createAssistantDraftStore,
  assistantDraftKey,
} from "./assistantDraftStore";
test("private/new/crew drafts stay isolated and account change clears and disposes retained values", () => {
  let epoch = 1,
    disposed = 0;
  const store = createAssistantDraftStore(() => epoch);
  const thread = assistantDraftKey("researcher", "thread");
  const home = assistantDraftKey(undefined, undefined, undefined, true);
  const compose = assistantDraftKey(undefined, undefined, "first");
  store.set(thread, "Thread draft");
  store.set(home, "Message NyxBot");
  store.set(compose, "New NyxBot chat");
  store.set("crew:one", { text: "Crew draft", recipients: ["a"] });
  store.set("uploads", {}, () => disposed++);
  assert.equal(store.get(thread), "Thread draft");
  assert.equal(store.get(home), "Message NyxBot");
  assert.equal(store.get(compose), "New NyxBot chat");
  assert.notEqual(compose, assistantDraftKey(undefined, undefined, "second"));
  assert.equal(thread, assistantDraftKey(undefined, "thread"));
  epoch++;
  assert.equal(store.get(thread), undefined);
  assert.equal(store.get(home), undefined);
  assert.equal(store.get(compose), undefined);
  assert.equal(store.get("crew:one"), undefined);
  assert.equal(disposed, 1);
});
