import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useDirectAssistantChat } from "@/hooks/use-assistant-direct";
import { directAssistantTransport } from "@/lib/assistant/direct-transport";
import { transitionAssistantIdentity } from "@/lib/assistant/identity";

beforeEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  transitionAssistantIdentity("direct-hook-user");
  directAssistantTransport.resetForIdentity("direct-hook-user");
});

describe("useDirectAssistantChat", () => {
  it("adopts a draft and presents its settled text through ChatSessionState", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        new Response(
          'data: {"choices":[{"delta":{"content":"Direct reply"},"finish_reason":"stop"}]}\n\ndata: [DONE]\n\n',
          { status: 200, headers: { "Content-Type": "text/event-stream" } },
        ),
      ),
    );
    const adopted = vi.fn();
    const { result, rerender } = renderHook(
      ({ conversationId }: { conversationId?: string }) =>
        useDirectAssistantChat({
          selectedConversationId: conversationId,
          onConversationAdopted: adopted,
        }),
      {
        initialProps: {
          conversationId: undefined as string | undefined,
        },
      },
    );

    await act(async () => result.current.send("Hello Direct"));
    const conversationId = adopted.mock.calls[0]?.[0] as string;
    rerender({ conversationId });

    expect(result.current.session).toMatchObject({
      conversationId,
      status: "completed_text",
      title: "Hello Direct",
    });
    expect(
      result.current.session.messages.map((message) => message.content),
    ).toEqual(["Hello Direct", "Direct reply"]);
  });

  it("maps local cancellation to a quiet stopped session", async () => {
    let streamController!: ReadableStreamDefaultController<Uint8Array>;
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        new Response(
          new ReadableStream<Uint8Array>({
            start(controller) {
              streamController = controller;
            },
            cancel() {
              streamController.close();
            },
          }),
          { status: 200, headers: { "Content-Type": "text/event-stream" } },
        ),
      ),
    );
    const conversationId = (await directAssistantTransport.createConversation()).id;
    const { result } = renderHook(() =>
      useDirectAssistantChat({
        selectedConversationId: conversationId,
        onConversationAdopted: vi.fn(),
      }),
    );

    let sendPromise: Promise<void> | undefined;
    act(() => {
      sendPromise = result.current.send("Stop this");
    });
    await waitFor(() => expect(result.current.isStreaming).toBe(true));
    await act(async () => result.current.stop());
    await act(async () => sendPromise);

    expect(result.current.session.status).toBe("stopped");
    expect(
      result.current.session.messages.some((message) => message.error),
    ).toBe(false);
  });
});
