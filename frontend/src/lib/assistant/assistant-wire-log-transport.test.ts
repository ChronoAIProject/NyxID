import { beforeEach, describe, expect, it, vi } from "vitest";
import { AevatarAssistantTransport, assistantApi } from "./aevatar-transport";
import {
  chatStreamClient,
  type ChatStreamRequestHandle,
} from "./chat-stream-worker-client";
import {
  ASSISTANT_WIRE_LOG_STORAGE_KEY,
  useAssistantWireLogStore,
} from "@/stores/assistant-wire-log-store";
import type { TurnEvent } from "@/types/assistant";

const ACTOR_ID = "nyxid-chat-f8369965a444433f92ec50e67ad8ee52";
const SECOND_ACTOR_ID = "nyxid-chat-4a1e60ebd1fd44f192bf4bb90e1812ae";
const WIRE_LOG_ID = "d7dbbf38-a31c-4331-8ddb-13fda5a70d12";

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
      featureEnabled: false,
      captureEnabled: false,
      showResponses: true,
      entries: [],
      totalBytes: 0,
      captureBytes: 0,
    });
  });

  it("suppresses the exact conversation list even when capture is enabled", async () => {
    const fetchMock = vi.fn().mockImplementation(() =>
      Promise.resolve(
        new Response(JSON.stringify({ conversations: [] }), {
          status: 200,
          headers: {
            "Content-Type": "application/json",
            "X-NyxID-Debug-Upstream-Id": WIRE_LOG_ID,
            "X-NyxID-Debug-Upstream-Log": encodedEcho(),
          },
        }),
      ),
    );
    vi.stubGlobal("fetch", fetchMock);
    useAssistantWireLogStore.getState().setFeatureEnabled(true);
    useAssistantWireLogStore.getState().setCaptureEnabled(true);

    await new AevatarAssistantTransport().listConversations();

    const init = fetchMock.mock.calls[0]?.[1] as RequestInit;
    expect(init.headers).not.toHaveProperty("X-NyxID-Debug-Upstream");
    expect(useAssistantWireLogStore.getState().entries).toEqual([]);
  });

  it("prefers the id header and attributes HTTP metadata from the endpoint", async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ status: "current" }), {
        status: 200,
        headers: {
          "Content-Type": "application/json",
          "X-NyxID-Debug-Upstream-Id": WIRE_LOG_ID,
          "X-NyxID-Debug-Upstream-Log": encodedEcho(),
        },
      }),
    );
    vi.stubGlobal("fetch", fetchMock);
    useAssistantWireLogStore.getState().setFeatureEnabled(true);
    useAssistantWireLogStore.getState().setCaptureEnabled(true);

    await assistantApi.get(`/assistant/conversations/${ACTOR_ID}/state`);

    const init = fetchMock.mock.calls[0]?.[1] as RequestInit;
    expect(init.headers).toMatchObject({
      "X-NyxID-Debug-Upstream": "1",
    });
    expect(useAssistantWireLogStore.getState().entries[0]).toMatchObject({
      kind: "header",
      status: 200,
      conversationId: ACTOR_ID,
      wireLogId: WIRE_LOG_ID,
      label: `GET /assistant/conversations/${ACTOR_ID}/state`,
    });
    expect(useAssistantWireLogStore.getState().entries[0]).not.toHaveProperty(
      "upstreamEchoes",
    );
    await vi.waitFor(() => {
      expect(useAssistantWireLogStore.getState().entries[0]?.capture).toEqual({
        state: "settled",
        outcome: "complete",
        wireOutcome: "complete",
        body: {
          text: JSON.stringify({ status: "current" }),
          bytes: 20,
          truncated: false,
        },
      });
    });
  });

  it("does not recursively capture a wire-log payload fetch", async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ id: WIRE_LOG_ID }), {
        status: 200,
        headers: {
          "Content-Type": "application/json",
          "X-NyxID-Debug-Upstream-Id": WIRE_LOG_ID,
        },
      }),
    );
    vi.stubGlobal("fetch", fetchMock);
    useAssistantWireLogStore.getState().setFeatureEnabled(true);
    useAssistantWireLogStore.getState().setCaptureEnabled(true);

    await assistantApi.get(`/assistant/wire-logs/${WIRE_LOG_ID}`);

    const init = fetchMock.mock.calls[0]?.[1] as RequestInit;
    expect(init.headers).not.toHaveProperty("X-NyxID-Debug-Upstream");
    expect(useAssistantWireLogStore.getState().entries).toEqual([]);
  });

  it("sends no debug header or stream observer when persisted capture meets a disabled feature", async () => {
    localStorage.setItem(
      ASSISTANT_WIRE_LOG_STORAGE_KEY,
      JSON.stringify({
        state: { captureEnabled: true, showResponses: true, entries: [] },
        version: 3,
      }),
    );
    await useAssistantWireLogStore.persist.rehydrate();
    expect(useAssistantWireLogStore.getState()).toMatchObject({
      featureEnabled: false,
      captureEnabled: true,
    });

    const fetchMock = vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ conversations: [] }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      }),
    );
    vi.stubGlobal("fetch", fetchMock);

    const transport = new AevatarAssistantTransport();
    await transport.listConversations();
    const listInit = fetchMock.mock.calls[0]?.[1] as RequestInit;
    expect(listInit.headers).not.toHaveProperty("X-NyxID-Debug-Upstream");

    const start = vi.spyOn(chatStreamClient, "start").mockImplementation(
      (request): ChatStreamRequestHandle => ({
        headers: Promise.resolve({
          kind: "response",
          status: 200,
          contentType: "text/event-stream",
        }),
        completion: Promise.resolve().then(() => {
          request.onFrames([
            {
              type: "RUN_STARTED",
              actorId: ACTOR_ID,
              turnId: "turn-wire-log-disabled",
            },
            { type: "RUN_FINISHED" },
          ]);
          return { kind: "complete" };
        }),
        cancel: vi.fn(),
      }),
    );
    const conversation = await transport.createConversation();
    await new Promise<void>((resolve) => {
      transport.sendMessage(conversation.id, "hello", (event) => {
        if (event.event === "turn.completed") resolve();
      });
    });

    expect(start).toHaveBeenCalledOnce();
    expect(start.mock.calls[0]?.[0].headers).toBeUndefined();
    expect(start.mock.calls[0]?.[0].onWire).toBeUndefined();
    expect(useAssistantWireLogStore.getState().entries).toEqual([]);
  });

  it("prefers the SSE id header and adopts the authoritative conversation", async () => {
    useAssistantWireLogStore.getState().setFeatureEnabled(true);
    useAssistantWireLogStore.getState().setCaptureEnabled(true);
    const start = vi.spyOn(chatStreamClient, "start").mockImplementation(
      (request): ChatStreamRequestHandle => ({
        headers: Promise.resolve({
          kind: "response",
          status: 200,
          contentType: "text/event-stream",
          debugUpstreamId: WIRE_LOG_ID,
          debugUpstream: encodedEcho(),
        }),
        completion: Promise.resolve().then(() => {
          request.onWire?.({
            type: "end",
            requestId: "id-backed-wire-log",
            outcome: "complete",
          });
          request.onFrames([
            {
              type: "RUN_STARTED",
              actorId: ACTOR_ID,
              turnId: "turn-id-backed-wire-log",
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

    expect(start.mock.calls[0]?.[0].headers).toEqual({
      "X-NyxID-Debug-Upstream": "1",
    });
    await vi.waitFor(() => {
      expect(useAssistantWireLogStore.getState().entries[0]).toMatchObject({
        kind: "sse",
        status: 200,
        conversationId: ACTOR_ID,
        wireLogId: WIRE_LOG_ID,
        label: "POST /assistant/chat",
        capture: {
          state: "settled",
          outcome: "complete",
          wireOutcome: "complete",
        },
      });
    });
    expect(useAssistantWireLogStore.getState().entries[0]).not.toHaveProperty(
      "upstreamEchoes",
    );
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
    useAssistantWireLogStore.getState().setFeatureEnabled(true);
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
              actorId: ACTOR_ID,
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
        wireOutcome: "complete",
        transportOutcome: "completed",
        framesSeen: 5,
        printableFramesSeen: 2,
        printableTurnEvents: 2,
        wireBytes: 48,
        terminalReceived: true,
        firstFrameMs: expect.any(Number),
        lastFrameMs: expect.any(Number),
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
    useAssistantWireLogStore.getState().setFeatureEnabled(true);
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
              actorId: ACTOR_ID,
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

  async function seedActorConversation(
    transport: AevatarAssistantTransport,
    conversationId: string,
  ): Promise<void> {
    useAssistantWireLogStore.getState().setCaptureEnabled(false);
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(
        JSON.stringify({ conversations: [{ id: conversationId }] }),
        {
          status: 200,
          headers: { "Content-Type": "application/json" },
        },
      ),
    );
    vi.stubGlobal("fetch", fetchMock);
    await transport.listConversations();
    useAssistantWireLogStore.getState().setCaptureEnabled(true);
  }

  it("L28 records clean EOF separately from stream-closed settlement", async () => {
    const conversationId = ACTOR_ID;
    useAssistantWireLogStore.getState().setFeatureEnabled(true);
    useAssistantWireLogStore.getState().setCaptureEnabled(true);
    const start = vi.spyOn(chatStreamClient, "start").mockImplementation(
      (request): ChatStreamRequestHandle => ({
        headers: Promise.resolve({
          kind: "response",
          status: 200,
          contentType: "text/event-stream",
          debugUpstream: encodedEcho(),
        }),
        completion: Promise.resolve().then(() => {
          request.onWire?.({
            type: "end",
            requestId: "clean-eof",
            outcome: "complete",
          });
          return { kind: "complete" };
        }),
        cancel: vi.fn(),
      }),
    );
    const transport = new AevatarAssistantTransport(() => 1_000);
    await seedActorConversation(transport, conversationId);

    const terminal = await new Promise<TurnEvent>((resolve) => {
      transport.sendMessage(conversationId, "hello", (event) => {
        if (event.event === "turn.completed") resolve(event);
      });
    });

    expect(terminal).toMatchObject({
      event: "turn.completed",
      status: "failed",
      error: { code: "stream_closed" },
    });
    expect(start).toHaveBeenCalledTimes(2);
    await vi.waitFor(() => {
      expect(
        useAssistantWireLogStore.getState().entries.at(-1)?.capture,
      ).toMatchObject({
        state: "settled",
        outcome: "complete",
        wireOutcome: "complete",
        transportOutcome: "stream_closed",
        framesSeen: 0,
        printableFramesSeen: 0,
        printableTurnEvents: 0,
        wireBytes: 0,
        terminalReceived: false,
        firstFrameMs: null,
        lastFrameMs: null,
      });
    });
  });

  it("L29 records a dying body as network error at both wire and transport layers", async () => {
    const conversationId = SECOND_ACTOR_ID;
    useAssistantWireLogStore.getState().setFeatureEnabled(true);
    useAssistantWireLogStore.getState().setCaptureEnabled(true);
    vi.spyOn(chatStreamClient, "start").mockImplementation(
      (request): ChatStreamRequestHandle => ({
        headers: Promise.resolve({
          kind: "response",
          status: 200,
          contentType: "text/event-stream",
          debugUpstream: encodedEcho(),
        }),
        completion: Promise.resolve().then(() => {
          request.onWire?.({
            type: "lines",
            requestId: "dying-body",
            lines: [{ text: "data:", ending: "\n" }],
            bytes: 6,
            truncated: false,
          });
          request.onWire?.({
            type: "end",
            requestId: "dying-body",
            outcome: "network_error",
          });
          request.onFrames([
            {
              type: "RUN_STARTED",
              actorId: conversationId,
              turnId: "turn-dying-body",
            },
          ]);
          return {
            kind: "network_error",
            code: "network_error",
            message: "body read failed",
          };
        }),
        cancel: vi.fn(),
      }),
    );
    const transport = new AevatarAssistantTransport(() => 1_025);
    await seedActorConversation(transport, conversationId);

    const terminal = await new Promise<TurnEvent>((resolve) => {
      transport.sendMessage(conversationId, "hello", (event) => {
        if (event.event === "turn.completed") resolve(event);
      });
    });

    expect(terminal).toMatchObject({
      event: "turn.completed",
      status: "failed",
      error: { code: "network_error" },
    });
    await vi.waitFor(() => {
      expect(
        useAssistantWireLogStore.getState().entries.at(-1)?.capture,
      ).toMatchObject({
        state: "settled",
        outcome: "network_error",
        wireOutcome: "network_error",
        transportOutcome: "network_error",
        framesSeen: 1,
        printableFramesSeen: 0,
        printableTurnEvents: 0,
        wireBytes: 6,
        terminalReceived: false,
        firstFrameMs: 0,
        lastFrameMs: 0,
      });
    });
  });
});
