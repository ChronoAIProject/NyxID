import { afterEach, describe, expect, it, vi } from "vitest";
import { AssistantTurnActiveError } from "@/lib/assistant/errors";
import capturedStream from "@/lib/assistant/__fixtures__/aevatar-workflow-chat-stream.sse?raw";
import { AevatarWorkflowChatTransport } from "@/lib/assistant/workflow-transport";
import type { TurnEvent } from "@/types/assistant";

const WORKFLOW_CHAT_URL = "/api/v1/assistant/workflow-chat";
// The final answer the captured run produced ("Say the single word: ping").
const CAPTURED_OUTPUT = "ping";

function streamResponse(body: string, chunkSize?: number): Response {
  const bytes = new TextEncoder().encode(body);
  const size = chunkSize ?? bytes.length;
  const stream = new ReadableStream<Uint8Array>({
    start(controller) {
      for (let i = 0; i < bytes.length; i += size) {
        controller.enqueue(bytes.slice(i, i + size));
      }
      controller.close();
    },
  });
  return new Response(stream, {
    status: 200,
    headers: { "Content-Type": "text/event-stream" },
  });
}

function stubStream(response: () => Response): ReturnType<typeof vi.fn> {
  const mock = vi.fn(() => Promise.resolve(response()));
  vi.stubGlobal("fetch", mock);
  return mock;
}

function collectTurn(
  transport: AevatarWorkflowChatTransport,
  conversationId: string,
  content: string,
): Promise<TurnEvent[]> {
  return new Promise((resolve, reject) => {
    const events: TurnEvent[] = [];
    try {
      transport.sendMessage(conversationId, content, (event) => {
        events.push(event);
        if (event.event === "turn.completed") resolve(events);
      });
    } catch (error) {
      reject(error instanceof Error ? error : new Error(String(error)));
    }
  });
}

async function newConversationId(
  transport: AevatarWorkflowChatTransport,
): Promise<string> {
  const conversation = await transport.createConversation();
  return conversation.id;
}

function terminalOf(events: TurnEvent[]) {
  const terminal = events[events.length - 1];
  return terminal?.event === "turn.completed"
    ? { status: terminal.status, error: terminal.error }
    : null;
}

afterEach(() => {
  vi.unstubAllGlobals();
});

// The fixture is a verbatim capture of the live `/api/chat` workflow-chat
// stream through the NyxID proxy (2026-07-17). These tests replay the REAL
// bytes — if aevatar changes its envelope shape, refresh the capture and
// these tests say exactly what broke.
describe("AevatarWorkflowChatTransport", () => {
  it("renders only the runFinished output from the captured engine stream", async () => {
    const fetchMock = stubStream(() => streamResponse(capturedStream));
    const transport = new AevatarWorkflowChatTransport();
    const id = await newConversationId(transport);

    const events = await collectTurn(transport, id, "Say the single word: ping");

    expect(terminalOf(events)).toEqual({ status: "completed", error: null });
    // The 20+ telemetry envelopes (run context, raw.observed, step events,
    // state snapshot) must produce exactly one assistant message.
    expect(events.map((event) => event.event)).toEqual([
      "turn.status",
      "message.started",
      "block.started",
      "block.delta",
      "block.completed",
      "message.completed",
      "turn.completed",
    ]);
    const delta = events.find((event) => event.event === "block.delta");
    expect(delta && "text" in delta ? delta.text : null).toBe(CAPTURED_OUTPUT);

    const request = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(request[0]).toBe(WORKFLOW_CHAT_URL);
    expect(JSON.parse(String(request[1].body))).toEqual({
      prompt: "Say the single word: ping",
    });
  });

  it("parses the captured stream identically under byte-level chunking", async () => {
    stubStream(() => streamResponse(capturedStream, 7));
    const transport = new AevatarWorkflowChatTransport();
    const id = await newConversationId(transport);

    const events = await collectTurn(transport, id, "Say the single word: ping");

    expect(terminalOf(events)).toEqual({ status: "completed", error: null });
    const history = await transport.getHistory(id);
    const assistant = history.messages.find((m) => m.role === "assistant");
    const text = assistant?.blocks
      .map((block) => (block.type === "text" ? block.text : ""))
      .join("");
    expect(text).toBe(CAPTURED_OUTPUT);
  });

  it("never addresses aevatar directly: no scope, proxy, or user id in the URL", async () => {
    const fetchMock = stubStream(() => streamResponse(capturedStream));
    const transport = new AevatarWorkflowChatTransport();
    const id = await newConversationId(transport);

    await collectTurn(transport, id, "hello");

    for (const call of fetchMock.mock.calls as [string, RequestInit][]) {
      expect(call[0]).not.toContain("/proxy/");
      expect(call[0]).not.toContain("/scopes/");
    }
  });

  it("completes without a message when runFinished has an empty output", async () => {
    stubStream(() =>
      streamResponse(
        'data: { "timestamp": "1", "runFinished": { "threadId": "t", "result": {} } }\n\n',
      ),
    );
    const transport = new AevatarWorkflowChatTransport();
    const id = await newConversationId(transport);

    const events = await collectTurn(transport, id, "hello");

    expect(terminalOf(events)).toEqual({ status: "completed", error: null });
    expect(events.map((event) => event.event)).toEqual([
      "turn.status",
      "turn.completed",
    ]);
  });

  it("fails the turn when the stream ends without a runFinished", async () => {
    stubStream(() =>
      streamResponse(
        'data: { "timestamp": "1", "custom": { "name": "aevatar.run.context", "payload": {} } }\n\n',
      ),
    );
    const transport = new AevatarWorkflowChatTransport();
    const id = await newConversationId(transport);

    const events = await collectTurn(transport, id, "hello");

    expect(terminalOf(events)).toEqual({
      status: "failed",
      error: {
        code: "stream_ended",
        message: "The workflow run ended without a result.",
      },
    });
  });

  it("fails the turn on a runError envelope", async () => {
    stubStream(() =>
      streamResponse(
        'data: { "timestamp": "1", "runError": { "code": "boom", "message": "It broke." } }\n\n',
      ),
    );
    const transport = new AevatarWorkflowChatTransport();
    const id = await newConversationId(transport);

    const events = await collectTurn(transport, id, "hello");

    expect(terminalOf(events)).toEqual({
      status: "failed",
      error: { code: "boom", message: "It broke." },
    });
  });

  it("fails the turn on a non-2xx response", async () => {
    stubStream(() => new Response("nope", { status: 502 }));
    const transport = new AevatarWorkflowChatTransport();
    const id = await newConversationId(transport);

    const events = await collectTurn(transport, id, "hello");

    expect(terminalOf(events)).toEqual({
      status: "failed",
      error: {
        code: "http_502",
        message: "The workflow chat stream could not be started.",
      },
    });
  });

  it("rejects a second send while a turn is active", async () => {
    stubStream(() => streamResponse(capturedStream));
    const transport = new AevatarWorkflowChatTransport();
    const id = await newConversationId(transport);

    const first = collectTurn(transport, id, "hello");
    expect(() => transport.sendMessage(id, "again", () => {})).toThrow(
      AssistantTurnActiveError,
    );
    await first;
  });

  it("cancel aborts the fetch and settles the turn as cancelled", async () => {
    // A stream that never closes: only telemetry, no runFinished.
    const stream = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(
          new TextEncoder().encode(
            'data: { "timestamp": "1", "custom": { "name": "aevatar.run.context", "payload": {} } }\n\n',
          ),
        );
      },
    });
    stubStream(
      () =>
        new Response(stream, {
          status: 200,
          headers: { "Content-Type": "text/event-stream" },
        }),
    );
    const transport = new AevatarWorkflowChatTransport();
    const id = await newConversationId(transport);

    const events: TurnEvent[] = [];
    const handle = transport.sendMessage(id, "hello", (event) => {
      events.push(event);
    });
    handle.cancel();

    expect(terminalOf(events)).toEqual({ status: "cancelled", error: null });
    // Cancelled turns must release the conversation for the next send.
    stubStream(() => streamResponse(capturedStream));
    const rerun = await collectTurn(transport, id, "hello again");
    expect(terminalOf(rerun)).toEqual({ status: "completed", error: null });
  });

  it("fails closed on a conversation id from another mode's world", async () => {
    const transport = new AevatarWorkflowChatTransport();
    await expect(transport.getHistory("completions-123")).rejects.toThrow(
      "Conversation was not found.",
    );
    expect(() => transport.sendMessage("nyxid-chat-1", "hi", () => {})).toThrow(
      "Conversation was not found.",
    );
  });

  it("refuses approvals: the workflow surface has none", async () => {
    const transport = new AevatarWorkflowChatTransport();
    await expect(transport.decideApproval()).rejects.toThrow(
      "Approvals are not available on the workflow chat API.",
    );
  });
});
