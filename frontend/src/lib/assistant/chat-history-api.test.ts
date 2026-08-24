import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  chatHistoryApi,
  ChatHistoryContractError,
} from "@/lib/assistant/chat-history-api";

const META = {
  id: "nyxid-chat-alpha",
  title: "Alpha",
  createdAt: "2026-08-24T00:00:00Z",
  updatedAt: "2026-08-24T00:01:00Z",
  messageCount: 2,
};

describe("chat history API", () => {
  beforeEach(() => {
    vi.unstubAllGlobals();
    globalThis.__nyxidAssistantHttpMock = undefined;
  });

  it("returns the fully drained NyxID conversation index", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        new Response(JSON.stringify({ conversations: [META] }), {
          status: 200,
          headers: { "Content-Type": "application/json" },
        }),
      ),
    );
    await expect(chatHistoryApi.listConversationMetas()).resolves.toEqual([META]);
  });

  it("rejects a non-null cursor instead of truncating the sidebar", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        new Response(
          JSON.stringify({ conversations: [META], nextCursor: "page-two" }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        ),
      ),
    );
    await expect(chatHistoryApi.listConversationMetas()).rejects.toEqual(
      expect.objectContaining<Partial<ChatHistoryContractError>>({
        name: "ChatHistoryContractError",
        path: "$index.nextCursor",
        message:
          "Invalid Chat History response at $index.nextCursor: expected no pagination from the NyxID drained index.",
      }),
    );
  });

  it("encodes the state cursor with version and turn identity", async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ status: "not_modified", stateVersion: 7 }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      }),
    );
    vi.stubGlobal("fetch", fetchMock);
    await chatHistoryApi.loadConversationState(
      "nyxid-chat-alpha",
      { afterStateVersion: 7, turnId: "turn-alpha" },
    );
    expect(fetchMock.mock.calls[0]?.[0]).toBe(
      "/api/v1/assistant/conversations/nyxid-chat-alpha/state?afterStateVersion=7&turnId=turn-alpha",
    );
  });
});
