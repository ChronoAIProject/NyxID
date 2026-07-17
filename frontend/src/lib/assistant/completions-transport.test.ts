import { afterEach, describe, expect, it, vi } from "vitest";
import { AevatarCompletionsTransport } from "@/lib/assistant/completions-transport";
import { AssistantTurnActiveError } from "@/lib/assistant/errors";
import capturedStream from "@/lib/assistant/__fixtures__/aevatar-chat-completions-stream.sse?raw";
import type { AssistantTransport, TurnEvent } from "@/types/assistant";

const COMPLETIONS_URL = "/api/v1/assistant/completions";
// The `id` every chunk of the captured stream carries; the transport derives
// the assistant message + block ids from it.
const CAPTURED_CHUNK_ID = "chatcmpl_xHmZTuDmJ00Wcmqve3pmMg";

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
  transport: AevatarCompletionsTransport,
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
  transport: AevatarCompletionsTransport,
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

// The fixture is a verbatim capture of the live `/v1/chat/completions`
// stream through the NyxID proxy (2026-07-17). These tests replay the REAL
// bytes — if aevatar changes its chunk shape, refresh the capture and these
// tests say exactly what broke.
describe("AevatarCompletionsTransport", () => {
  it("adapts the captured completions stream into the PRD turn-event sequence", async () => {
    stubStream(() => streamResponse(capturedStream));
    const transport = new AevatarCompletionsTransport();
    const id = await newConversationId(transport);

    const events = await collectTurn(transport, id, "Say completions stream ok");

    expect(events.map((event) => event.event)).toEqual([
      "turn.status",
      "message.started",
      "block.started",
      "block.delta",
      "block.delta",
      "block.delta",
      "block.delta",
      "block.delta",
      "block.completed",
      "message.completed",
      "turn.completed",
    ]);
    expect(
      events
        .filter((event) => event.event === "block.delta")
        .map((event) => (event.event === "block.delta" ? event.text : "")),
    ).toEqual(["com", "plet", "ions", " stream", " ok"]);
    const completed = events.find((event) => event.event === "block.completed");
    expect(completed?.event === "block.completed" && completed.block).toEqual({
      type: "text",
      block_id: `${CAPTURED_CHUNK_ID}-text`,
      text: "completions stream ok",
    });
    expect(terminalOf(events)).toEqual({ status: "completed", error: null });

    // Cursors are per-turn, start at 1, and strictly increase.
    const cursors = events.map((event) => event.cursor);
    expect(cursors[0]).toBe(1);
    expect(cursors).toEqual([...cursors].sort((a, b) => a - b));
    expect(new Set(cursors).size).toBe(cursors.length);
  });

  it("parses the captured stream fed in awkward slices", async () => {
    // 7-byte chunks split `data:` prefixes, JSON payloads, and the \n\n frame
    // boundary mid-sequence — the incremental parser must not care.
    stubStream(() => streamResponse(capturedStream, 7));
    const transport = new AevatarCompletionsTransport();
    const id = await newConversationId(transport);

    const events = await collectTurn(transport, id, "Say completions stream ok");

    const completed = events.find((event) => event.event === "block.completed");
    expect(completed?.event === "block.completed" && completed.block).toEqual({
      type: "text",
      block_id: `${CAPTURED_CHUNK_ID}-text`,
      text: "completions stream ok",
    });
    expect(terminalOf(events)).toEqual({ status: "completed", error: null });
  });

  it("sends the request shape the completions endpoint requires, history first", async () => {
    const fetchMock = stubStream(() => streamResponse(capturedStream));
    const transport = new AevatarCompletionsTransport();
    const id = await newConversationId(transport);
    await collectTurn(transport, id, "First question");

    await collectTurn(transport, id, "Second question");

    const lastCall = fetchMock.mock.calls[fetchMock.mock.calls.length - 1] as
      | [string, RequestInit]
      | undefined;
    expect(lastCall).toBeDefined();
    const [url, init] = lastCall ?? ["", {}];
    expect(url).toBe(COMPLETIONS_URL);
    expect(init.method).toBe("POST");
    expect(init.credentials).toBe("include");
    expect(init.headers).toMatchObject({
      "Content-Type": "application/json",
      Accept: "text/event-stream",
    });
    const body = JSON.parse(String(init.body)) as Record<string, unknown>;
    // Stateless endpoint: the prior turn rides along, new message last, and
    // `model` is left to the server default.
    expect(body).toEqual({
      stream: true,
      messages: [
        { role: "user", content: "First question" },
        { role: "assistant", content: "completions stream ok" },
        { role: "user", content: "Second question" },
      ],
    });
    expect(body).not.toHaveProperty("model");
  });

  it("serves the local transcript from getHistory after the turn", async () => {
    stubStream(() => streamResponse(capturedStream));
    const transport = new AevatarCompletionsTransport();
    const id = await newConversationId(transport);
    await collectTurn(transport, id, "Say completions stream ok");

    const history = await transport.getHistory(id);

    expect(history.has_more).toBe(false);
    expect(history.conversation.title).toBe("Say completions stream ok");
    expect(history.messages).toHaveLength(2);
    expect(history.messages[0]?.role).toBe("user");
    expect(history.messages[1]?.blocks).toEqual([
      {
        type: "text",
        block_id: `${CAPTURED_CHUNK_ID}-text`,
        text: "completions stream ok",
      },
    ]);
    // Conversation ids are disjoint from the nyxid-chat transport's.
    expect(id.startsWith("completions-")).toBe(true);
    const list = await transport.listConversations();
    expect(list.map((item) => item.id)).toEqual([id]);
  });

  it("settles a completed turn when the stream carries only [DONE]", async () => {
    stubStream(() => streamResponse("data: [DONE]\n\n"));
    const transport = new AevatarCompletionsTransport();
    const id = await newConversationId(transport);

    const events = await collectTurn(transport, id, "Hello");

    expect(events.map((event) => event.event)).toEqual([
      "turn.status",
      "turn.completed",
    ]);
    expect(terminalOf(events)).toEqual({ status: "completed", error: null });
  });

  it("settles a completed turn when every delta is empty", async () => {
    stubStream(() =>
      streamResponse(
        [
          `data: ${JSON.stringify({
            id: "chatcmpl_empty",
            choices: [{ index: 0, delta: {}, finish_reason: null }],
          })}\n\n`,
          `data: ${JSON.stringify({
            id: "chatcmpl_empty",
            choices: [{ index: 0, delta: {}, finish_reason: "stop" }],
          })}\n\n`,
          "data: [DONE]\n\n",
        ].join(""),
      ),
    );
    const transport = new AevatarCompletionsTransport();
    const id = await newConversationId(transport);

    const events = await collectTurn(transport, id, "Hello");

    expect(events.map((event) => event.event)).toEqual([
      "turn.status",
      "turn.completed",
    ]);
    expect(terminalOf(events)).toEqual({ status: "completed", error: null });
  });

  it("skips unparseable and unknown frames rather than dropping the turn", async () => {
    stubStream(() =>
      streamResponse(
        [
          "data: not-json-at-all\n\n",
          `data: ${JSON.stringify({ object: "some.future.frame" })}\n\n`,
          `data: ${JSON.stringify({
            id: "chatcmpl_ok",
            choices: [{ index: 0, delta: { content: "hi" }, finish_reason: null }],
          })}\n\n`,
          "data: [DONE]\n\n",
        ].join(""),
      ),
    );
    const transport = new AevatarCompletionsTransport();
    const id = await newConversationId(transport);

    const events = await collectTurn(transport, id, "Hello");

    const completed = events.find((event) => event.event === "block.completed");
    expect(completed?.event === "block.completed" && completed.block).toEqual({
      type: "text",
      block_id: "chatcmpl_ok-text",
      text: "hi",
    });
    expect(terminalOf(events)).toEqual({ status: "completed", error: null });
  });

  it("maps an error frame to a failed turn", async () => {
    stubStream(() =>
      streamResponse(
        `data: ${JSON.stringify({
          error: { code: "upstream_timeout", message: "Model timed out" },
        })}\n\n`,
      ),
    );
    const transport = new AevatarCompletionsTransport();
    const id = await newConversationId(transport);

    const events = await collectTurn(transport, id, "Hello");

    expect(terminalOf(events)).toEqual({
      status: "failed",
      error: { code: "upstream_timeout", message: "Model timed out" },
    });
  });

  it("defaults the error code when the error frame omits it", async () => {
    stubStream(() =>
      streamResponse(
        `data: ${JSON.stringify({ error: { message: "Something broke" } })}\n\n`,
      ),
    );
    const transport = new AevatarCompletionsTransport();
    const id = await newConversationId(transport);

    const events = await collectTurn(transport, id, "Hello");

    expect(terminalOf(events)).toEqual({
      status: "failed",
      error: { code: "completions_error", message: "Something broke" },
    });
  });

  it("fails the turn when the endpoint rejects the request", async () => {
    stubStream(
      () =>
        new Response(JSON.stringify({ error: "boom" }), {
          status: 500,
          headers: { "Content-Type": "application/json" },
        }),
    );
    const transport = new AevatarCompletionsTransport();
    const id = await newConversationId(transport);

    const events = await collectTurn(transport, id, "Hello");

    expect(terminalOf(events)).toEqual({
      status: "failed",
      error: {
        code: "http_500",
        message: "The assistant stream could not be started.",
      },
    });
  });

  it("rejects a concurrent send while a turn is active", async () => {
    stubStream(() => streamResponse(capturedStream));
    const transport = new AevatarCompletionsTransport();
    const id = await newConversationId(transport);

    const first = collectTurn(transport, id, "First message");
    expect(() => {
      transport.sendMessage(id, "Second message", () => {});
    }).toThrow(AssistantTurnActiveError);
    await first;
  });

  it("settles open blocks and aborts the fetch on cancel", async () => {
    // A stream that emits one content chunk then stays open forever.
    let signal: AbortSignal | undefined;
    const fetchMock = vi.fn((_url: RequestInfo | URL, init?: RequestInit) => {
      signal = init?.signal ?? undefined;
      const open = new ReadableStream<Uint8Array>({
        start(controller) {
          controller.enqueue(
            new TextEncoder().encode(
              `data: ${JSON.stringify({
                id: "chatcmpl_open",
                choices: [
                  { index: 0, delta: { content: "par" }, finish_reason: null },
                ],
              })}\n\n`,
            ),
          );
        },
      });
      return Promise.resolve(
        new Response(open, {
          status: 200,
          headers: { "Content-Type": "text/event-stream" },
        }),
      );
    });
    vi.stubGlobal("fetch", fetchMock);
    const transport = new AevatarCompletionsTransport();
    const id = await newConversationId(transport);

    const events: TurnEvent[] = [];
    await new Promise<void>((resolve) => {
      const handle = transport.sendMessage(id, "Hello", (event) => {
        events.push(event);
        if (event.event === "turn.completed") resolve();
        if (event.event === "block.delta") handle.cancel();
      });
    });

    expect(events.map((event) => event.event)).toEqual([
      "turn.status",
      "message.started",
      "block.started",
      "block.delta",
      "block.completed",
      "message.completed",
      "turn.status",
      "turn.completed",
    ]);
    expect(terminalOf(events)).toEqual({ status: "cancelled", error: null });
    expect(signal?.aborted).toBe(true);
    // Cancel-flow leaves the partial text in the transcript.
    const history = await transport.getHistory(id);
    expect(history.messages[1]?.blocks).toEqual([
      { type: "text", block_id: "chatcmpl_open-text", text: "par" },
    ]);
  });

  it("throws for unknown conversations and for approvals", async () => {
    // Through the interface: this is how the delegator and hooks call it.
    const transport: AssistantTransport = new AevatarCompletionsTransport();

    // A nyxid-chat id lands here only after a mode toggle — it must fail
    // closed rather than resolve against the wrong world.
    await expect(transport.getHistory("nyxid-chat-elsewhere")).rejects.toThrow(
      "Conversation was not found.",
    );
    expect(() => {
      transport.sendMessage("nyxid-chat-elsewhere", "Hello", () => {});
    }).toThrow("Conversation was not found.");
    await expect(
      transport.decideApproval("whatever", "block-1", true),
    ).rejects.toThrow("Approvals are not available on the completions API.");
  });

  it("rejects an empty or oversized message", async () => {
    const transport = new AevatarCompletionsTransport();
    const id = await newConversationId(transport);

    expect(() => {
      transport.sendMessage(id, "   ", () => {});
    }).toThrow("Message must contain between 1 and 32768 characters.");
    expect(() => {
      transport.sendMessage(id, "x".repeat(32_769), () => {});
    }).toThrow("Message must contain between 1 and 32768 characters.");
  });
});
