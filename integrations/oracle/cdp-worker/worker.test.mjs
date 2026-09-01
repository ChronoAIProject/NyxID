import assert from "node:assert/strict";
import test from "node:test";

import {
  backoffDelay,
  choosePromptNavigation,
  decidePromptResume,
  decryptSessionEnvelope,
  markChatPageRecovered,
  taskRecoveryDecision,
} from "./worker.mjs";

// Produced by Rust encrypt_login_snapshot for LOGIN_SNAPSHOT_FIXTURE_TOKEN.
// This cross-language wire fixture must never be regenerated silently.
const LOGIN_SNAPSHOT_FIXTURE_TOKEN = "nyx_owk_test-token-material";
const LOGIN_SNAPSHOT_FIXTURE = Buffer.from(
  '{"ciphertext_base64":"Wze5nFAvcqWAmAeGSzUc4agtvr4N9FH7L7CuwOTGynfBLTEV8ylIUuJ9gw7jJezU93XwSp8L6Q==","nonce_base64":"eo6ntLWmG/Ff7bVJ","salt_base64":"vJWSgK3y0bixEBTVH63n9xBLbUrXYY29SrxSWrZGg+c=","version":1}'
);

test("backoff is capped and jitter remains within the selected window", () => {
  assert.equal(backoffDelay(0, 100, 1000, () => 0), 50);
  assert.equal(backoffDelay(3, 100, 1000, () => 1), 800);
  assert.equal(backoffDelay(20, 100, 1000, () => 1), 1000);
});

test("a pre-send task may send only after the transcript is ready", () => {
  assert.deepEqual(
    decidePromptResume({
      phase: "claimed",
      prompt: "new question",
      turns: [],
      generating: false,
      transcriptReady: false,
    }),
    { action: "wait" }
  );
  assert.deepEqual(
    decidePromptResume({
      phase: "page_ready",
      prompt: "new question",
      turns: [],
      generating: false,
      transcriptReady: true,
    }),
    { action: "send" }
  );
});

test("recovery extracts a completed answer without resending", () => {
  assert.deepEqual(
    decidePromptResume({
      phase: "waiting_response",
      prompt: "What is 2 + 2?",
      turns: [
        { role: "user", text: "What is 2 + 2?" },
        { role: "assistant", text: "4" },
      ],
      generating: false,
      transcriptReady: true,
    }),
    { action: "complete", response: "4" }
  );
});

test("recovery waits when the sent prompt exists but has no settled answer", () => {
  assert.deepEqual(
    decidePromptResume({
      phase: "sent",
      prompt: "long answer",
      turns: [{ role: "user", text: "long answer" }],
      generating: true,
      transcriptReady: true,
    }),
    { action: "wait" }
  );
});

test("a persisted send attempt with no matching turn is never resent", () => {
  assert.deepEqual(
    decidePromptResume({
      phase: "send_attempted",
      prompt: "repeat me",
      turns: [],
      generating: false,
      transcriptReady: true,
    }),
    { action: "uncertain" }
  );
});

test("recovery preserves a live conversation when only the project root was persisted", () => {
  assert.deepEqual(
    choosePromptNavigation({
      recovering: true,
      phase: "send_attempted",
      isFollowup: false,
      currentUrl: "https://chatgpt.com/c/12345678-abcd",
      persistedUrl: "https://chatgpt.com/g/g-project/example/project",
      taskConversationUrl: null,
      requiredProjectUrl: "https://chatgpt.com/g/g-project/example/project",
    }),
    { error: null, target: null }
  );
});

test("pre-send recovery leaves an unrelated conversation before sending", () => {
  assert.deepEqual(
    choosePromptNavigation({
      recovering: true,
      phase: "page_ready",
      isFollowup: false,
      currentUrl: "https://chatgpt.com/c/12345678-abcd",
      persistedUrl: "https://chatgpt.com/g/g-project/example/project",
      taskConversationUrl: null,
      requiredProjectUrl: "https://chatgpt.com/g/g-project/example/project",
    }),
    {
      error: null,
      target: "https://chatgpt.com/g/g-project/example/project",
    }
  );
});

test("pre-send recovery without a conversation safely returns to the project", () => {
  assert.deepEqual(
    choosePromptNavigation({
      recovering: true,
      phase: "claimed",
      isFollowup: false,
      currentUrl: "https://chatgpt.com/",
      persistedUrl: null,
      taskConversationUrl: null,
      requiredProjectUrl: "https://chatgpt.com/g/g-project/example/project",
    }),
    {
      error: null,
      target: "https://chatgpt.com/g/g-project/example/project",
    }
  );
});

test("recovery navigates to a known conversation instead of an unrelated live tab", () => {
  assert.deepEqual(
    choosePromptNavigation({
      recovering: true,
      phase: "sent",
      isFollowup: true,
      currentUrl: "https://chatgpt.com/c/aaaaaaaa-bbbb",
      persistedUrl: "https://chatgpt.com/c/cccccccc-dddd",
      taskConversationUrl: "https://chatgpt.com/c/cccccccc-dddd",
      requiredProjectUrl: null,
    }),
    { error: null, target: "https://chatgpt.com/c/cccccccc-dddd" }
  );
});

test("a server-pinned conversation outranks a persisted project root", () => {
  assert.deepEqual(
    choosePromptNavigation({
      recovering: true,
      phase: "waiting_response",
      isFollowup: true,
      currentUrl: "https://chatgpt.com/",
      persistedUrl: "https://chatgpt.com/g/g-project/example/project",
      taskConversationUrl: "https://chatgpt.com/c/cccccccc-dddd",
      requiredProjectUrl: "https://chatgpt.com/g/g-project/example/project",
    }),
    { error: null, target: "https://chatgpt.com/c/cccccccc-dddd" }
  );
});

test("recovery fails closed when neither state nor the live tab identifies a conversation", () => {
  assert.deepEqual(
    choosePromptNavigation({
      recovering: true,
      phase: "send_attempted",
      isFollowup: false,
      currentUrl: "https://chatgpt.com/",
      persistedUrl: null,
      taskConversationUrl: null,
      requiredProjectUrl: null,
    }),
    { error: "recovery_conversation_unknown", target: null }
  );
});

test("the transcript baseline ignores an identical prompt from an older turn", () => {
  assert.deepEqual(
    decidePromptResume({
      phase: "send_attempted",
      prompt: "same prompt",
      turns: [
        { role: "user", text: "same prompt" },
        { role: "assistant", text: "old answer" },
      ],
      generating: false,
      transcriptReady: true,
      baselineTurnCount: 2,
    }),
    { action: "uncertain" }
  );
});

test("task recovery relaunches Chrome and then stops at a bounded threshold", () => {
  assert.deepEqual(
    taskRecoveryDecision({
      kind: "prompt",
      phase: "page_ready",
      failureCount: 3,
      maxFailures: 6,
      relaunchEvery: 3,
    }),
    { action: "recover", forceRelaunch: true }
  );
  assert.deepEqual(
    taskRecoveryDecision({
      kind: "prompt",
      phase: "page_ready",
      failureCount: 6,
      maxFailures: 6,
      relaunchEvery: 3,
    }),
    { action: "fail", code: "browser_recovery_exhausted" }
  );
});

test("post-send recovery exhaustion never authorizes a prompt retry", () => {
  assert.deepEqual(
    taskRecoveryDecision({
      kind: "prompt",
      phase: "send_attempted",
      failureCount: 6,
      maxFailures: 6,
      relaunchEvery: 3,
    }),
    { action: "fail", code: "prompt_delivery_uncertain" }
  );
  assert.deepEqual(
    taskRecoveryDecision({
      kind: "scrape",
      phase: "extracting",
      failureCount: 6,
      maxFailures: 6,
      relaunchEvery: 3,
    }),
    { action: "fail", code: "browser_recovery_exhausted" }
  );
});

test("a Rust login snapshot fixture decrypts with only its pool token", () => {
  const expected = { version: 1, cookies: [], origins: [] };

  assert.deepEqual(
    decryptSessionEnvelope(LOGIN_SNAPSHOT_FIXTURE, LOGIN_SNAPSHOT_FIXTURE_TOKEN),
    expected
  );
  assert.throws(
    () => decryptSessionEnvelope(LOGIN_SNAPSHOT_FIXTURE, "nyx_owk_wrong"),
    /session_decrypt_failed/
  );
});

test("idle Chrome recovery clears stale health errors", () => {
  const idle = {
    state: { current_task: null },
    health: { tab: 2 },
    chromeAlive: false,
    lastError: "cdp_connection_refused",
  };
  markChatPageRecovered(idle);
  assert.equal(idle.health.tab, 0);
  assert.equal(idle.chromeAlive, true);
  assert.equal(idle.lastError, null);

  const active = {
    state: { current_task: { task_id: "task-1" } },
    health: { tab: 2 },
    chromeAlive: false,
    lastError: "cdp_connection_refused",
  };
  markChatPageRecovered(active);
  assert.equal(active.lastError, "cdp_connection_refused");
});
