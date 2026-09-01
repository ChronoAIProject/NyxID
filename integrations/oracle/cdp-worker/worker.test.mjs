import assert from "node:assert/strict";
import { createCipheriv, hkdfSync, randomBytes } from "node:crypto";
import test from "node:test";

import {
  backoffDelay,
  choosePromptNavigation,
  decidePromptResume,
  decryptSessionEnvelope,
  taskRecoveryDecision,
} from "./worker.mjs";

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

test("session envelope decrypts with the pool token and rejects another token", () => {
  const token = "nyx_owk_test-token-material";
  const salt = randomBytes(32);
  const nonce = randomBytes(12);
  const info = Buffer.from("nyxid-oracle-session-v1");
  const key = Buffer.from(hkdfSync("sha256", Buffer.from(token), salt, info, 32));
  const cipher = createCipheriv("aes-256-gcm", key, nonce);
  cipher.setAAD(info);
  const expected = { version: 1, cookies: [], origins: [] };
  const body = Buffer.concat([
    cipher.update(Buffer.from(JSON.stringify(expected))),
    cipher.final(),
  ]);
  const envelope = Buffer.from(
    JSON.stringify({
      version: 1,
      salt_base64: salt.toString("base64"),
      nonce_base64: nonce.toString("base64"),
      ciphertext_base64: Buffer.concat([body, cipher.getAuthTag()]).toString("base64"),
    })
  );

  assert.deepEqual(decryptSessionEnvelope(envelope, token), expected);
  assert.throws(
    () => decryptSessionEnvelope(envelope, "nyx_owk_wrong"),
    /session_decrypt_failed/
  );
});
