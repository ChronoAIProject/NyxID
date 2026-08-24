import { describe, expect, it } from "vitest";
import capturedHistory from "@/lib/assistant/__fixtures__/aevatar-chat-history.json";
import { hydrateStoredMessages } from "@/lib/assistant/chat-session-state";

import {
  ChatHistoryApiError,
  ChatHistoryContractError,
  decodeChatConversationDetail,
  decodeChatHistoryIndex,
} from "./chat-history-decoders";

describe("chat history decoders", () => {
  it("decodes one fully drained NyxID conversation page with all metadata", () => {
    expect(
      decodeChatHistoryIndex({
        conversations: [
          {
            activeStepSummary: "Connect GitHub",
            attentionKind: "action",
            attentionSince: "2026-08-04T02:35:00+00:00",
            createdAt: "2026-08-04T02:30:00+00:00",
            id: "nyxid-chat-alpha",
            llmModel: "gpt-5.4-mini",
            llmRoute: "/api/v1/proxy/s/openai",
            messageCount: 2,
            serviceId: "nyxid-chat",
            serviceKind: "nyxid-chat",
            stateVersion: 7,
            taskStatus: "blocked",
            title: "New conversation",
            updatedAt: "2026-08-04T02:35:00+00:00",
          },
        ],
      }),
    ).toEqual({
      conversations: [
        {
          activeStepSummary: "Connect GitHub",
          attentionKind: "action",
          attentionSince: "2026-08-04T02:35:00+00:00",
          createdAt: "2026-08-04T02:30:00+00:00",
          id: "nyxid-chat-alpha",
          llmModel: "gpt-5.4-mini",
          llmRoute: "/api/v1/proxy/s/openai",
          messageCount: 2,
          serviceId: "nyxid-chat",
          serviceKind: "nyxid-chat",
          stateVersion: 7,
          taskStatus: "blocked",
          title: "New conversation",
          updatedAt: "2026-08-04T02:35:00+00:00",
        },
      ],
    });
  });

  it("preserves nullable actor attention metadata", () => {
    expect(
      decodeChatHistoryIndex({
        conversations: [
          {
            activeStepSummary: null,
            attentionKind: null,
            attentionSince: null,
            createdAt: "2026-08-04T02:30:00+00:00",
            id: "nyxid-chat-idle",
            llmModel: null,
            llmRoute: null,
            messageCount: 0,
            taskStatus: null,
            title: "Idle conversation",
            updatedAt: "2026-08-04T02:35:00+00:00",
          },
        ],
      }).conversations[0],
    ).toMatchObject({
      activeStepSummary: null,
      attentionKind: null,
      attentionSince: null,
      llmModel: null,
      llmRoute: null,
      taskStatus: null,
    });
  });

  it("preserves the upstream cursor so the NyxID API layer can reject it", () => {
    expect(
      decodeChatHistoryIndex({
        conversations: [],
        nextCursor: "unexpected-page",
      }),
    ).toEqual({ conversations: [], nextCursor: "unexpected-page" });
    expect(
      decodeChatHistoryIndex({ conversations: [], nextCursor: null }),
    ).toEqual({ conversations: [], nextCursor: null });
  });

  it("preserves transcript fields, turn identity, and extensible strings", () => {
    expect(
      decodeChatConversationDetail({
        messages: [
          {
            authorId: null,
            authorName: "Automation",
            content: "Queued for review",
            error: null,
            id: "turn-a:observer",
            role: "observer",
            status: "queued",
            thinking: "Checking policy",
            timestamp: 1784255700000,
            turnId: "turn-a",
          },
        ],
        projectionStatus: "current",
        stateVersion: 7,
      }),
    ).toEqual({
      messages: [
        {
          authorId: null,
          authorName: "Automation",
          content: "Queued for review",
          error: null,
          id: "turn-a:observer",
          role: "observer",
          status: "queued",
          thinking: "Checking policy",
          timestamp: 1784255700000,
          turnId: "turn-a",
        },
      ],
      projectionStatus: "current",
      stateVersion: 7,
    });
  });

  it("preserves pending projection status", () => {
    expect(
      decodeChatConversationDetail({
        messages: [],
        projectionStatus: "pending",
        stateVersion: 0,
      }),
    ).toEqual({ messages: [], projectionStatus: "pending", stateVersion: 0 });
  });

  it("hydrates a stored completed message as settled from the wrapped capture", () => {
    const detail = decodeChatConversationDetail(capturedHistory);

    expect(detail.projectionStatus).toBe("current");
    expect(hydrateStoredMessages(detail.messages)[1]).toMatchObject({
      content: "Blue is a color.  \nGreen is a color.",
      role: "assistant",
      status: "complete",
    });
  });

  it("rejects malformed successful index rows and transcripts with paths", () => {
    expect(() => decodeChatHistoryIndex({ conversations: {} })).toThrow(
      ChatHistoryContractError,
    );
    expect(() =>
      decodeChatHistoryIndex({
        conversations: [
          {
            createdAt: "2026-08-04T02:30:00+00:00",
            id: "nyxid-chat-alpha",
            messageCount: -1,
            title: "Invalid",
            updatedAt: "2026-08-04T02:35:00+00:00",
          },
        ],
      }),
    ).toThrow(
      expect.objectContaining({ path: "$index.conversations[0].messageCount" }),
    );
    expect(() =>
      decodeChatHistoryIndex({
        conversations: [
          {
            createdAt: "2026-08-04T02:30:00+00:00",
            id: "nyxid-chat-alpha",
            messageCount: 1,
            stateVersion: "7",
            title: "Invalid",
            updatedAt: "2026-08-04T02:35:00+00:00",
          },
        ],
      }),
    ).toThrow(
      expect.objectContaining({ path: "$index.conversations[0].stateVersion" }),
    );
    expect(() => decodeChatConversationDetail([])).toThrow(
      ChatHistoryContractError,
    );
    expect(() =>
      decodeChatConversationDetail({
        messages: [],
        projectionStatus: "stale",
        stateVersion: 0,
      }),
    ).toThrow(
      expect.objectContaining({ path: "$conversation.projectionStatus" }),
    );
    expect(() =>
      decodeChatConversationDetail({
        messages: [{ id: "missing-fields" }],
        projectionStatus: "current",
        stateVersion: 0,
      }),
    ).toThrow(
      expect.objectContaining({ path: "$conversation.messages[0].content" }),
    );
  });

  it("retains the structured API error contract for the HTTP layer", () => {
    expect(
      new ChatHistoryApiError("Denied", 403, "ACCESS_DENIED"),
    ).toMatchObject({
      code: "ACCESS_DENIED",
      message: "Denied",
      status: 403,
    });
  });
});
