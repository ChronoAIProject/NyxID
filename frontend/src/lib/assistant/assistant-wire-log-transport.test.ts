import { beforeEach, describe, expect, it, vi } from "vitest";
import { AevatarAssistantTransport } from "./aevatar-transport";
import {
  chatStreamClient,
  type ChatStreamRequestHandle,
} from "./chat-stream-worker-client";
import { useAssistantWireLogStore } from "@/stores/assistant-wire-log-store";
import type { TurnEvent } from "@/types/assistant";

function encodedEcho(): string {
  const json = JSON.stringify([
    {
      method: "GET",
      path: "api/chat/conversations",
      commandType: null,
      body: null,
      headers: { "content-type": "application/json" },
      identity: {
        mode: "jwt",
        forward_access_token: false,
        inject_delegation_token: true,
        bridge_minted: false,
        futureIdentityMetadata: "ignored",
      },
      truncated: false,
      futureEnvelopeMetadata: "ignored",
    },
  ]);
  return btoa(String.fromCharCode(...new TextEncoder().encode(json)));
}

describe("assistant wire-log transport", () => {
  beforeEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    localStorage.clear();
    useAssistantWireLogStore.setState({
      captureEnabled: false,
      showResponses: true,
      entries: [],
      totalBytes: 0,
      captureBytes: 0,
    });
  });

  it("sends the gate only when enabled and decodes header delivery", async () => {
    const fetchMock = vi.fn().mockImplementation(() =>
      Promise.resolve(
        new Response(JSON.stringify({ conversations: [] }), {
          status: 200,
          headers: {
            "Content-Type": "application/json",
            "X-NyxID-Debug-Upstream-Log": encodedEcho(),
          },
        }),
      ),
    );
    vi.stubGlobal("fetch", fetchMock);

    await new AevatarAssistantTransport().listConversations();
    const firstInit = fetchMock.mock.calls[0]?.[1] as RequestInit;
    expect(firstInit.headers).not.toHaveProperty("X-NyxID-Debug-Upstream");
    expect(useAssistantWireLogStore.getState().entries).toHaveLength(0);

    useAssistantWireLogStore.getState().setCaptureEnabled(true);
    await new AevatarAssistantTransport().listConversations();
    const secondInit = fetchMock.mock.calls[1]?.[1] as RequestInit;
    expect(secondInit.headers).toMatchObject({
      "X-NyxID-Debug-Upstream": "1",
    });
    expect(useAssistantWireLogStore.getState().entries[0]).toMatchObject({
      kind: "header",
      status: 200,
      upstreamEchoes: [{ method: "GET", path: "api/chat/conversations" }],
    });
    expect(useAssistantWireLogStore.getState().entries[0]).not.toHaveProperty(
      "futureEnvelopeMetadata",
    );
    const capturedEcho =
      useAssistantWireLogStore.getState().entries[0]?.upstreamEchoes[0];
    if (!capturedEcho || capturedEcho.degraded) {
      throw new Error("expected a full legacy echo");
    }
    expect(capturedEcho.identity).not.toHaveProperty("futureIdentityMetadata");
    expect(capturedEcho).not.toHaveProperty("futureEnvelopeMetadata");
    await vi.waitFor(() => {
      expect(useAssistantWireLogStore.getState().entries[0]?.capture).toEqual({
        state: "settled",
        outcome: "complete",
        body: {
          text: JSON.stringify({ conversations: [] }),
          bytes: 20,
          truncated: false,
        },
      });
    });
  });

  it("buffers wire data until its backend echo arrives without delivering it as chat content", async () => {
    const debugOnlyMarker = "wire-log-only-prompt";
    const rawWireMarker = "raw-wire-line-only";
    const debugUpstream = btoa(
      String.fromCharCode(
        ...new TextEncoder().encode(
          JSON.stringify([
            {
              method: "POST",
              path: "api/chat",
              commandType: "text",
              body: { type: "text", prompt: debugOnlyMarker },
              headers: {
                accept: "text/event-stream",
                "content-type": "application/json",
              },
              identity: {
                mode: "jwt",
                forward_access_token: false,
                inject_delegation_token: true,
                bridge_minted: false,
              },
              truncated: false,
            },
          ]),
        ),
      ),
    );
    useAssistantWireLogStore.getState().setCaptureEnabled(true);
    let resolveHeaders:
      | ((value: Awaited<ChatStreamRequestHandle["headers"]>) => void)
      | undefined;
    const headers = new Promise<Awaited<ChatStreamRequestHandle["headers"]>>(
      (resolve) => {
        resolveHeaders = resolve;
      },
    );
    const start = vi.spyOn(chatStreamClient, "start").mockImplementation(
      (request): ChatStreamRequestHandle => ({
        headers,
        completion: Promise.resolve().then(() => {
          request.onWire?.({
            type: "lines",
            requestId: "worker-request-1",
            lines: [
              {
                text: `data: {"debug":"${rawWireMarker}"}`,
                ending: "\r\n",
              },
              { text: "", ending: "\r\n" },
            ],
            bytes: 48,
            truncated: false,
          });
          request.onWire?.({
            type: "end",
            requestId: "worker-request-1",
            outcome: "complete",
          });
          resolveHeaders?.({
            kind: "response",
            status: 200,
            contentType: "text/event-stream",
            debugUpstream,
          });
          request.onFrames([
            {
              type: "RUN_STARTED",
              actorId: "nyxid-chat-wire-log",
              turnId: "turn-wire-log",
            },
            {
              type: "TEXT_MESSAGE_START",
              textMessageStart: {
                messageId: "message-wire-log",
                role: "assistant",
              },
            },
            {
              type: "TEXT_MESSAGE_CONTENT",
              textMessageContent: { delta: "Visible assistant reply" },
            },
            {
              type: "TEXT_MESSAGE_END",
              textMessageEnd: { messageId: "message-wire-log" },
            },
            { type: "RUN_FINISHED" },
          ]);
          return { kind: "complete" };
        }),
        cancel: vi.fn(),
      }),
    );
    const transport = new AevatarAssistantTransport();
    const conversation = await transport.createConversation();

    const events = await new Promise<TurnEvent[]>((resolve) => {
      const delivered: TurnEvent[] = [];
      transport.sendMessage(conversation.id, "hello", (event) => {
        delivered.push(event);
        if (event.event === "turn.completed") resolve(delivered);
      });
    });

    expect(start).toHaveBeenCalledOnce();
    expect(start.mock.calls[0]?.[0].headers).toEqual({
      "X-NyxID-Debug-Upstream": "1",
    });
    await vi.waitFor(() => {
      expect(useAssistantWireLogStore.getState().entries[0]).toMatchObject({
        kind: "sse",
        status: 200,
        upstreamEchoes: [
          {
            method: "POST",
            path: "api/chat",
            commandType: "text",
            body: { type: "text", prompt: debugOnlyMarker },
          },
        ],
      });
    });
    expect(JSON.stringify(events)).toContain("Visible assistant reply");
    expect(JSON.stringify(events)).not.toContain(debugOnlyMarker);
    expect(JSON.stringify(events)).not.toContain(rawWireMarker);
    await vi.waitFor(() => {
      expect(useAssistantWireLogStore.getState().entries[0]?.capture).toEqual({
        state: "settled",
        outcome: "complete",
        sse: {
          lines: [
            {
              text: `data: {"debug":"${rawWireMarker}"}`,
              ending: "\r\n",
            },
            { text: "", ending: "\r\n" },
          ],
          bytes: 48,
          retainedBytes: 40,
          truncated: false,
        },
      });
    });
  });

  it("discards buffered wire data when the backend returned no echo", async () => {
    useAssistantWireLogStore.getState().setCaptureEnabled(true);
    const start = vi.spyOn(chatStreamClient, "start").mockImplementation(
      (request): ChatStreamRequestHandle => ({
        headers: Promise.resolve({
          kind: "response",
          status: 200,
          contentType: "text/event-stream",
        }),
        completion: Promise.resolve().then(() => {
          request.onWire?.({
            type: "lines",
            requestId: "worker-request-without-echo",
            lines: [{ text: "data: private", ending: "\n" }],
            bytes: 14,
            truncated: false,
          });
          request.onWire?.({
            type: "end",
            requestId: "worker-request-without-echo",
            outcome: "complete",
          });
          request.onFrames([
            {
              type: "RUN_STARTED",
              actorId: "nyxid-chat-wire-log",
              turnId: "turn-without-echo",
            },
            { type: "RUN_FINISHED" },
          ]);
          return { kind: "complete" };
        }),
        cancel: vi.fn(),
      }),
    );

    const transport = new AevatarAssistantTransport();
    const conversation = await transport.createConversation();
    await new Promise<void>((resolve) => {
      transport.sendMessage(conversation.id, "hello", (event) => {
        if (event.event === "turn.completed") resolve();
      });
    });

    expect(start).toHaveBeenCalledOnce();
    expect(useAssistantWireLogStore.getState().entries).toEqual([]);
  });
});
