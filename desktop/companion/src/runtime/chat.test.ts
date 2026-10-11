import { describe, expect, it } from "vitest";

import {
  nyxIdChatCommandErrorSchema,
  nyxIdChatEventSchema,
  nyxIdChatHistorySchema,
  nyxIdChatRecoverySchema,
  nyxIdChatRecoveryDelayMs,
  nyxIdChatRequestSchema,
} from "./chat";

const conversationId = "nyxa-0123456789abcdef0123456789abcdef";
const requestId = "123e4567-e89b-42d3-a456-426614174000";

describe("NyxID companion chat boundary", () => {
  it("accepts the public request and streamed event shapes", () => {
    expect(
      nyxIdChatRequestSchema.parse({
        requestId,
        conversationId,
        text: "请通过 NyxID 帮我处理",
      }),
    ).toEqual({ requestId, conversationId, text: "请通过 NyxID 帮我处理" });

    expect(
      nyxIdChatEventSchema.parse({
        kind: "started",
        requestId,
        conversationId,
        turnId: "turn-1",
      }),
    ).toMatchObject({ kind: "started", conversationId });

    expect(
      nyxIdChatEventSchema.parse({
        kind: "snapshot",
        requestId,
        text: "完整回复",
      }),
    ).toEqual({ kind: "snapshot", requestId, text: "完整回复" });
  });

  it("accepts only the minimal public history projection", () => {
    const history = {
      conversation: { id: conversationId, activeTurn: false },
      messages: [
        {
          id: "message-1",
          seq: 1,
          turnId: "turn-1",
          role: "assistant",
          text: "处理完成",
          status: "completed",
          errorCode: null,
        },
      ],
    };

    expect(nyxIdChatHistorySchema.parse(history)).toEqual(history);
    expect(() =>
      nyxIdChatHistorySchema.parse({
        ...history,
        accessToken: "must-not-cross-ipc",
      }),
    ).toThrow();
    expect(() =>
      nyxIdChatHistorySchema.parse({
        ...history,
        messages: [{ ...history.messages[0], serviceSlug: "api-lark" }],
      }),
    ).toThrow();

    expect(nyxIdChatRecoverySchema.parse({ requestId, history })).toEqual({
      requestId,
      history,
    });
    expect(() =>
      nyxIdChatRecoverySchema.parse({
        requestId,
        history,
        ownerUserId: "must-not-cross-ipc",
      }),
    ).toThrow();
  });

  it("rejects blank messages and malformed conversation ids", () => {
    expect(() =>
      nyxIdChatRequestSchema.parse({ requestId, text: "   " }),
    ).toThrow();
    expect(() =>
      nyxIdChatRequestSchema.parse({
        requestId,
        conversationId: "not-a-conversation",
        text: "hello",
      }),
    ).toThrow();
    expect(() =>
      nyxIdChatRequestSchema.parse({
        requestId: "123e4567-e89b-12d3-a456-426614174000",
        text: "hello",
      }),
    ).toThrow();
  });

  it("accepts only the public command-error fields", () => {
    expect(
      nyxIdChatCommandErrorSchema.parse({
        kind: "rejected",
        message: "这条消息没有执行。",
      }),
    ).toEqual({ kind: "rejected", message: "这条消息没有执行。" });
    expect(() =>
      nyxIdChatCommandErrorSchema.parse({
        kind: "rejected",
        message: "这条消息没有执行。",
        accessToken: "must-not-cross-ipc",
      }),
    ).toThrow();
  });

  it("backs recovery off with bounded deterministic jitter", () => {
    expect(nyxIdChatRecoveryDelayMs(0, () => 1)).toBe(750);
    expect(nyxIdChatRecoveryDelayMs(1, () => 0)).toBe(1_125);
    expect(nyxIdChatRecoveryDelayMs(1, () => 1)).toBe(1_500);
    expect(nyxIdChatRecoveryDelayMs(2, () => 0)).toBe(2_250);
    expect(nyxIdChatRecoveryDelayMs(2, () => 1)).toBe(3_000);
    expect(nyxIdChatRecoveryDelayMs(4, () => 0)).toBe(7_500);
    expect(nyxIdChatRecoveryDelayMs(99, () => 1)).toBe(10_000);
  });

  it("clamps malformed recovery inputs", () => {
    expect(nyxIdChatRecoveryDelayMs(-4, () => 1)).toBe(750);
    expect(nyxIdChatRecoveryDelayMs(Number.NaN, () => 1)).toBe(750);
    expect(nyxIdChatRecoveryDelayMs(Number.POSITIVE_INFINITY, () => 2)).toBe(
      10_000,
    );
    expect(nyxIdChatRecoveryDelayMs(2, () => Number.NaN)).toBe(2_250);
    expect(
      nyxIdChatRecoveryDelayMs(2, () => {
        throw new Error("random unavailable");
      }),
    ).toBe(2_250);
  });
});
