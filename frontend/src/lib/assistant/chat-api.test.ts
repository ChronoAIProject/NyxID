import { beforeEach, describe, expect, it, vi } from "vitest";
import { sendChatCommand, ChatApiError } from "@/lib/assistant/chat-api";

describe("chat API", () => {
  beforeEach(() => {
    vi.unstubAllGlobals();
    globalThis.__nyxidAssistantHttpMock = undefined;
  });

  it("sends the typed command with the idempotency key", async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response("data: [DONE]\n\n", {
        status: 200,
        headers: { "Content-Type": "text/event-stream" },
      }),
    );
    vi.stubGlobal("fetch", fetchMock);
    await sendChatCommand(
      {
        type: "text",
        clientRequestId: " request-1 ",
        prompt: " hello ",
      },
      new AbortController().signal,
    );

    const init = fetchMock.mock.calls[0]?.[1] as RequestInit;
    expect(init.headers).toMatchObject({
      Accept: "text/event-stream",
      "Idempotency-Key": "request-1",
    });
    expect(JSON.parse(String(init.body))).toEqual({
      type: "text",
      clientRequestId: "request-1",
      prompt: "hello",
    });
  });

  it("converts a non-OK response before the stream reader is entered", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        new Response(
          JSON.stringify({
            error: "session_expired",
            error_code: 2001,
            message: "Session expired",
          }),
          { status: 401, headers: { "Content-Type": "application/json" } },
        ),
      ),
    );

    await expect(
      sendChatCommand(
        { type: "text", clientRequestId: "request-2", prompt: "hello" },
        new AbortController().signal,
      ),
    ).rejects.toEqual(
      expect.objectContaining<Partial<ChatApiError>>({
        name: "ChatApiError",
        message: "Session expired",
        status: 401,
        code: 2001,
      }),
    );
  });
});
