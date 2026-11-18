import assert from "node:assert/strict";
import test from "node:test";
import {
  AssistantSseDecoder,
  applyAssistantEvent,
  emptyLiveReply,
  liveReplyText,
} from "./assistantStream";
import type { AssistantTurnEvent } from "./assistantTypes";

const conversationId = `nyxa-${"a".repeat(32)}`;
const event = (data: AssistantTurnEvent) =>
  `data: ${JSON.stringify(data)}\r\n\r\n`;

test("SSE parser preserves split CRLF, comments, and split text frames", () => {
  const decoder = new AssistantSseDecoder();
  const payload = `: keepalive\r\n\r\n${event({ event: "turn.status", cursor: 1, conversation_id: conversationId, turn_id: "turn-1", status: "running" })}${event({ event: "block.delta", cursor: 2, block_id: "b1", text: "你好\n世界" })}`;
  const events: AssistantTurnEvent[] = [];
  for (const character of payload) events.push(...decoder.push(character));
  assert.equal(events.length, 2);
  assert.equal(events[0]?.event, "turn.status");
  assert.deepEqual(events[1], {
    event: "block.delta",
    cursor: 2,
    block_id: "b1",
    text: "你好\n世界",
  });
});

test("SSE parser supports multi-line data and ignores unknown additive events", () => {
  const decoder = new AssistantSseDecoder();
  const result = decoder.push(
    'data: {"event":"future.event","cursor":1}\n\ndata: {"event":"message.started",\ndata: "cursor":2,"message_id":"m1","role":"assistant"}\n\n',
  );
  assert.deepEqual(result, [
    {
      event: "message.started",
      cursor: 2,
      message_id: "m1",
      role: "assistant",
    },
  ]);
});

test("recognized malformed frames and oversized events fail closed", () => {
  assert.throws(() =>
    new AssistantSseDecoder().push(
      'data: {"event":"turn.status","cursor":1,"conversation_id":"wrong"}\n\n',
    ),
  );
  assert.throws(
    () =>
      new AssistantSseDecoder().push(`data: ${"x".repeat(1024 * 1024)}\n\n`),
    /exceeded/,
  );
});

test("stream reducer adopts server identity and authoritative completed text", () => {
  let state = applyAssistantEvent(emptyLiveReply(), {
    event: "turn.status",
    cursor: 1,
    conversation_id: conversationId,
    turn_id: "turn-1",
    status: "running",
  });
  state = applyAssistantEvent(state, {
    event: "message.started",
    cursor: 2,
    message_id: "m1",
    role: "assistant",
  });
  state = applyAssistantEvent(state, {
    event: "block.started",
    cursor: 3,
    message_id: "m1",
    block_id: "b1",
    index: 0,
    block: { type: "text", block_id: "b1", text: "" },
  });
  state = applyAssistantEvent(state, {
    event: "block.delta",
    cursor: 4,
    block_id: "b1",
    text: "partial",
  });
  assert.equal(liveReplyText(state), "partial");
  state = applyAssistantEvent(state, {
    event: "block.completed",
    cursor: 5,
    block_id: "b1",
    block: { type: "text", block_id: "b1", text: "完整结果" },
  });
  state = applyAssistantEvent(state, {
    event: "turn.completed",
    cursor: 6,
    turn_id: "turn-1",
    status: "completed",
    error: null,
  });
  assert.equal(state.conversationId, conversationId);
  assert.equal(state.messageId, "m1");
  assert.equal(state.terminal, "completed");
  assert.equal(liveReplyText(state), "完整结果");
});

test("stream replay cursor is ignored and cross-conversation or turn data is refused", () => {
  const start: AssistantTurnEvent = {
    event: "turn.status",
    cursor: 1,
    conversation_id: conversationId,
    turn_id: "turn-1",
    status: "running",
  };
  const state = applyAssistantEvent(emptyLiveReply(), start);
  assert.equal(applyAssistantEvent(state, start), state);
  assert.throws(
    () =>
      applyAssistantEvent(state, {
        ...start,
        cursor: 2,
        conversation_id: `nyxa-${"b".repeat(32)}`,
      }),
    /conversation mismatch/,
  );
  assert.throws(
    () =>
      applyAssistantEvent(state, {
        event: "turn.completed",
        cursor: 2,
        turn_id: "turn-2",
        status: "completed",
        error: null,
      }),
    /turn mismatch/,
  );
  assert.throws(
    () =>
      applyAssistantEvent(state, {
        event: "block.delta",
        cursor: 2,
        block_id: "missing",
        text: "wrong",
      }),
    /not started/,
  );
  assert.throws(
    () =>
      applyAssistantEvent(state, {
        event: "block.started",
        cursor: 2,
        message_id: "wrong-message",
        block_id: "b1",
        index: 0,
        block: { type: "text", block_id: "b1", text: "wrong" },
      }),
    /identity mismatch/,
  );
});
