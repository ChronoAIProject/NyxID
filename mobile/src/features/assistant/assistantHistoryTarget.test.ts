import assert from "node:assert/strict";
import test from "node:test";
import {
  acknowledgementForConversation,
  assistantHistoryRequestPath,
  selectAssistantHistoryTarget,
} from "./assistantHistoryTarget";

const conversationId = `nyxa-${"a".repeat(32)}`;
const otherConversationId = `nyxa-${"b".repeat(32)}`;
const acknowledgementId = "d9428888-122b-11e1-b85c-61cd3cbb3210";
const otherAcknowledgementId = "f47ac10b-58cc-4372-a567-0e02b2c3d479";
const current = { conversationId, acknowledgementId };

test("history requests preserve the existing path when no card is selected", () => {
  const path = `/assistant/nyxagent/conversations/${conversationId}?limit=50`;
  assert.equal(assistantHistoryRequestPath(conversationId), path);
  assert.equal(assistantHistoryRequestPath(conversationId, 0), path);
  assert.equal(
    assistantHistoryRequestPath(conversationId, 42),
    `${path}&before_seq=42`,
  );
});

test("history requests include both the older-page cursor and the target card", () => {
  assert.equal(
    assistantHistoryRequestPath(conversationId, 42, acknowledgementId),
    `/assistant/nyxagent/conversations/${conversationId}?limit=50&before_seq=42&card=${acknowledgementId}`,
  );
  assert.equal(
    assistantHistoryRequestPath(conversationId, undefined, acknowledgementId),
    `/assistant/nyxagent/conversations/${conversationId}?limit=50&card=${acknowledgementId}`,
  );
});

test("history requests reject malformed conversation IDs before constructing a URL", () => {
  for (const id of [
    "",
    "nyxa-invalid",
    `${conversationId}/other`,
    "../other",
  ]) {
    assert.throws(() => assistantHistoryRequestPath(id));
  }
});

test("history requests reject malformed acknowledgement IDs", () => {
  for (const id of ["", "not-a-uuid", `${acknowledgementId}&card=other`]) {
    assert.throws(() =>
      assistantHistoryRequestPath(conversationId, undefined, id),
    );
  }
});

test("reloading the same conversation retains its selected acknowledgement", () => {
  assert.deepEqual(
    selectAssistantHistoryTarget(current, conversationId),
    current,
  );
});

test("explicitly selecting another acknowledgement replaces the old target", () => {
  assert.deepEqual(
    selectAssistantHistoryTarget(
      current,
      conversationId,
      otherAcknowledgementId,
    ),
    { conversationId, acknowledgementId: otherAcknowledgementId },
  );
});

test("switching conversations clears the previous acknowledgement", () => {
  assert.deepEqual(selectAssistantHistoryTarget(current, otherConversationId), {
    conversationId: otherConversationId,
  });
});

test("a new conversation can supply its own acknowledgement", () => {
  assert.deepEqual(
    selectAssistantHistoryTarget(
      current,
      otherConversationId,
      otherAcknowledgementId,
    ),
    {
      conversationId: otherConversationId,
      acknowledgementId: otherAcknowledgementId,
    },
  );
});

test("opening a conversation without an existing target starts without a card", () => {
  assert.deepEqual(selectAssistantHistoryTarget(undefined, conversationId), {
    conversationId,
  });
});

test("clearing the conversation also clears its acknowledgement target", () => {
  assert.equal(selectAssistantHistoryTarget(current), undefined);
  assert.equal(
    selectAssistantHistoryTarget(current, undefined, acknowledgementId),
    undefined,
  );
  assert.equal(selectAssistantHistoryTarget(current, ""), undefined);
});

test("acknowledgements are returned only for their exact conversation", () => {
  assert.equal(
    acknowledgementForConversation(current, conversationId),
    acknowledgementId,
  );
  assert.equal(
    acknowledgementForConversation(current, otherConversationId),
    undefined,
  );
  assert.equal(
    acknowledgementForConversation(undefined, conversationId),
    undefined,
  );
  assert.equal(
    acknowledgementForConversation({ conversationId }, conversationId),
    undefined,
  );
});
