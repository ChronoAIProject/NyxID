import assert from "node:assert/strict";
import test from "node:test";
import {
  assistantTurnBody,
  restoreRefusedAssistantSend,
} from "./assistantTurnRequest";
const id = "nyxa-" + "a".repeat(32);
const file = "11111111-1111-4111-8111-111111111111";
test("attachment-only send uses its existing upload draft and omits new-chat identity", () => {
  assert.deepEqual(
    assistantTurnBody({
      conversationId: id,
      agentId: "agent",
      text: "",
      attachmentIds: [file],
    }),
    { conversation_id: id, text: "", attachment_ids: [file] },
  );
});
test("empty text needs files or an attachment-only retry", () => {
  assert.throws(() => assistantTurnBody({ text: "  " }));
  assert.deepEqual(
    assistantTurnBody({ conversationId: id, text: "", retryOf: "turn" }),
    { conversation_id: id, text: "", retry_of: "turn" },
  );
});
test("retry cannot resend already-bound attachment IDs", () => {
  assert.throws(() =>
    assistantTurnBody({
      conversationId: id,
      text: "",
      retryOf: "turn",
      attachmentIds: [file],
    }),
  );
});
test("attachments require an exact conversation, distinct UUIDs and a bounded count", () => {
  assert.throws(() => assistantTurnBody({ text: "hi", attachmentIds: [file] }));
  assert.throws(() =>
    assistantTurnBody({
      conversationId: id,
      text: "hi",
      attachmentIds: [file, file],
    }),
  );
  assert.throws(() =>
    assistantTurnBody({
      conversationId: id,
      text: "hi",
      attachmentIds: ["malformed"],
    }),
  );
  assert.throws(() =>
    assistantTurnBody({ conversationId: "other", text: "hi" }),
  );
});
test("only a definite refusal before admission can restore editable text and uploads", () => {
  assert.equal(restoreRefusedAssistantSend({ statusCode: 400 }, false), true);
  assert.equal(restoreRefusedAssistantSend({ statusCode: 409 }, true), false);
  assert.equal(restoreRefusedAssistantSend({ statusCode: 503 }, false), false);
  assert.equal(restoreRefusedAssistantSend(new Error("timeout"), false), false);
});

test("native new turns use the server default and strip legacy selection fields", () => {
  const input = {
    text: "Hello",
    agentId: "specialist",
    model: "nyxagent/research",
  };
  assert.deepEqual(assistantTurnBody(input), {
    agent_id: "specialist",
    text: "Hello",
  });
  assert.deepEqual(assistantTurnBody({ ...input, conversationId: id }), {
    conversation_id: id,
    text: "Hello",
  });
});
