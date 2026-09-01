import assert from "node:assert/strict";
import { createCipheriv, hkdfSync, randomBytes } from "node:crypto";
import test from "node:test";

import {
  backoffDelay,
  decidePromptResume,
  decryptSessionEnvelope,
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
