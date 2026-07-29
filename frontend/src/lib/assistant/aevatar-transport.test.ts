import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  AevatarAssistantTransport,
  redactDisplayText,
  summarizeToolResult,
} from "@/lib/assistant/aevatar-transport";
import { AssistantTurnActiveError } from "@/lib/assistant/errors";
import { selectAssistantTransportKind } from "@/lib/assistant/transport";
import capturedHistory from "@/lib/assistant/__fixtures__/aevatar-chat-history.json";
import capturedStream from "@/lib/assistant/__fixtures__/aevatar-nyxid-chat-stream.sse?raw";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
import type { ContentBlock, TurnEvent } from "@/types/assistant";

const USER_ID = "add69059-bece-4f0e-9559-99cfd10b47eb";
const CONVERSATION_ID = "nyxid-chat-f8369965a444433f92ec50e67ad8ee52";
const TURN_ID = "turn-server-owned-1";
// NyxID's own assistant mount. No scope segment: the server derives the
// aevatar scope from the session user (PRD decision 4).
const ASSISTANT_BASE = "/api/v1/assistant";

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

function sseResponse(frames: unknown[]): Response {
  const body = frames
    .map((frame) => `data: ${JSON.stringify(frame)}\n\n`)
    .join("");
  return new Response(body, {
    status: 200,
    headers: { "Content-Type": "text/event-stream" },
  });
}

function chunkedSseResponse(frameChunks: unknown[][]): Response {
  const encoder = new TextEncoder();
  const chunks = frameChunks.map((frames) =>
    encoder.encode(
      frames.map((frame) => `data: ${JSON.stringify(frame)}\n\n`).join(""),
    ),
  );
  let chunkIndex = 0;
  return new Response(
    new ReadableStream<Uint8Array>({
      pull(controller) {
        const chunk = chunks[chunkIndex];
        chunkIndex += 1;
        if (chunk) controller.enqueue(chunk);
        else controller.close();
      },
    }),
    {
      status: 200,
      headers: { "Content-Type": "text/event-stream" },
    },
  );
}

// The exact frame sequence observed live against aevatar's
// `nyxid-chat/conversations/{id}:stream` on 2026-07-16.
const OBSERVED_FRAMES = [
  { type: "RUN_STARTED", turnId: TURN_ID, actorId: CONVERSATION_ID },
  {
    type: "TEXT_MESSAGE_START",
    textMessageStart: { messageId: "m-1", role: "assistant" },
  },
  { type: "TEXT_MESSAGE_CONTENT", textMessageContent: { delta: "Hello, " } },
  {
    type: "TEXT_MESSAGE_CONTENT",
    textMessageContent: { delta: "hope your day shines." },
  },
  {
    type: "USAGE",
    usage: { available: true, promptTokens: 25444, completionTokens: 24 },
  },
  { type: "TEXT_MESSAGE_END", textMessageEnd: { messageId: "m-1" } },
  { type: "RUN_FINISHED" },
];

type FetchRoute = (
  url: string,
  init: RequestInit | undefined,
) => Response | undefined;

function stubFetch(...routes: FetchRoute[]): ReturnType<typeof vi.fn> {
  const mock = vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    for (const route of routes) {
      const response = route(url, init);
      if (response) return Promise.resolve(response);
    }
    return Promise.resolve(
      jsonResponse({ error: "not_found", error_code: -1, message: "404" }, 404),
    );
  });
  vi.stubGlobal("fetch", mock);
  return mock;
}

const routeCreate: FetchRoute = (url, init) =>
  url === `${ASSISTANT_BASE}/conversations` && init?.method === "POST"
    ? jsonResponse({ status: "accepted", actorId: CONVERSATION_ID })
    : undefined;

function routeStream(frames: unknown[]): FetchRoute {
  return (url, init) =>
    url.endsWith("/stream") && init?.method === "POST"
      ? sseResponse(frames)
      : undefined;
}

// `body` is the whole transcript response, not just its entries: the reader
// accepts the legacy flat array and the PR #2923 `{messages, stateVersion}`
// wrapper, and must reject anything else.
function routeHistory(body: unknown): FetchRoute {
  return (url, init) =>
    url.startsWith(`${ASSISTANT_BASE}/conversations/`) &&
    (init?.method ?? "GET") === "GET"
      ? jsonResponse(body)
      : undefined;
}

function collectTurn(
  transport: AevatarAssistantTransport,
  content: string,
): Promise<TurnEvent[]> {
  return new Promise((resolve, reject) => {
    const events: TurnEvent[] = [];
    try {
      transport.sendMessage(CONVERSATION_ID, content, (event) => {
        events.push(event);
        if (event.event === "turn.completed") resolve(events);
      });
    } catch (error) {
      reject(error instanceof Error ? error : new Error(String(error)));
    }
  });
}

beforeEach(() => {
  useAuthStore.getState().setUser({ id: USER_ID } as User);
});

afterEach(() => {
  vi.unstubAllGlobals();
  useAuthStore.getState().setUser(null);
});

describe("AevatarAssistantTransport", () => {
  it("adapts the observed AG-UI stream into the PRD turn-event sequence", async () => {
    stubFetch(routeCreate, routeStream(OBSERVED_FRAMES));
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    const events = await collectTurn(transport, "Say hello in five words.");

    expect(events.map((event) => event.event)).toEqual([
      "turn.status",
      "message.started",
      "block.started",
      "block.delta",
      "block.delta",
      "block.completed",
      "message.completed",
      "turn.completed",
    ]);
    const cursors = events.map((event) => event.cursor);
    expect(cursors).toEqual([...cursors].sort((a, b) => a - b));
    expect(new Set(cursors).size).toBe(cursors.length);

    const completed = events.find((event) => event.event === "block.completed");
    expect(completed?.event === "block.completed" && completed.block).toEqual({
      type: "text",
      block_id: "m-1-text",
      text: "Hello, hope your day shines.",
    });
    const terminal = events[events.length - 1];
    expect(
      terminal?.event === "turn.completed" && {
        status: terminal.status,
        error: terminal.error,
      },
    ).toEqual({ status: "completed", error: null });
  });

  it("serves the streamed transcript from the local mirror during and after the turn", async () => {
    stubFetch(routeCreate, routeStream(OBSERVED_FRAMES), routeHistory([]));
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();
    await collectTurn(transport, "Say hello in five words.");

    // Server history is empty (materialization lag) — the keep-max guard
    // must retain the richer local transcript.
    const history = await transport.getHistory(CONVERSATION_ID);
    expect(history.messages).toHaveLength(2);
    expect(history.messages[0]?.role).toBe("user");
    expect(history.messages[1]?.role).toBe("assistant");
    expect(history.messages[1]?.blocks).toEqual([
      {
        type: "text",
        block_id: "m-1-text",
        text: "Hello, hope your day shines.",
      },
    ]);
    expect(history.conversation.title).toBe("Say hello in five words.");
  });

  it("maps the flat server history into text-block messages", async () => {
    stubFetch(
      routeHistory([
        {
          id: "e1-user",
          role: "user",
          content: "What services are connected?",
          timestamp: 1784192889074,
        },
        {
          id: "e1-assistant",
          role: "assistant",
          content: "You have GitHub and OpenAI connected.",
          timestamp: 1784192899074,
        },
      ]),
    );
    const transport = new AevatarAssistantTransport();

    const history = await transport.getHistory(CONVERSATION_ID);

    expect(history.conversation.title).toBe("What services are connected?");
    expect(history.has_more).toBe(false);
    expect(history.messages).toEqual([
      {
        id: "e1-user",
        role: "user",
        schema_version: 1,
        blocks: [
          {
            type: "text",
            block_id: "e1-user-text",
            text: "What services are connected?",
          },
        ],
        created_at: new Date(1784192889074).toISOString(),
      },
      {
        id: "e1-assistant",
        role: "assistant",
        schema_version: 1,
        blocks: [
          {
            type: "text",
            block_id: "e1-assistant-text",
            text: "You have GitHub and OpenAI connected.",
          },
        ],
        created_at: new Date(1784192899074).toISOString(),
      },
    ]);
  });

  it("preserves validated status, error, and server turnId from history", async () => {
    stubFetch(
      routeHistory([
        {
          id: "turn-history:assistant",
          role: "assistant",
          content: "Connect GitHub to continue.",
          timestamp: 1784192899074,
          status: "blocked",
          error: {
            code: "NYXID_UNAUTHORIZED",
            message: "Credential token=secret-value expired.",
          },
          turnId: "turn-history",
        },
      ]),
    );
    const transport = new AevatarAssistantTransport();

    const history = await transport.getHistory(CONVERSATION_ID);

    expect(history.messages[0]).toMatchObject({
      turnId: "turn-history",
      status: "blocked",
      error: {
        code: "NYXID_UNAUTHORIZED",
        message: 'Credential token="[redacted]" expired.',
      },
    });
  });

  it("uses RUN_STARTED.turnId as the authoritative handle and event identity", async () => {
    stubFetch(routeCreate, routeStream(OBSERVED_FRAMES));
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    const events: TurnEvent[] = [];
    let resolveDone: () => void = () => {};
    const done = new Promise<void>((resolve) => {
      resolveDone = resolve;
    });
    const handle = transport.sendMessage(CONVERSATION_ID, "Hello", (event) => {
      events.push(event);
      if (event.event === "turn.completed") resolveDone();
    });

    expect(handle.turnId).toBeNull();
    await done;
    expect(handle.turnId).toBe(TURN_ID);
    expect(
      events
        .filter(
          (event) =>
            event.event === "turn.status" || event.event === "turn.completed",
        )
        .map((event) => event.turn_id),
    ).toEqual([TURN_ID, TURN_ID]);
  });

  it("lists conversations from the Chat History index without a fan-out", async () => {
    const detailFetch = vi.fn();
    stubFetch(
      (url, init) =>
        url === `${ASSISTANT_BASE}/conversations` &&
        (init?.method ?? "GET") === "GET"
          ? jsonResponse({
              conversations: [
                {
                  id: CONVERSATION_ID,
                  title: "Summarize this week's merged PRs",
                  serviceId: "nyxid-chat",
                  serviceKind: "nyxid.chat",
                  createdAt: "2026-07-17T03:00:00+00:00",
                  updatedAt: "2026-07-17T03:05:00+00:00",
                  messageCount: 4,
                  llmRoute: "nyxid",
                  llmModel: "gpt-5.5",
                },
              ],
            })
          : undefined,
      // If the list ever hydrates titles via per-conversation detail reads,
      // this route fires and the assertion below fails.
      (url, init) => {
        if (
          url.startsWith(`${ASSISTANT_BASE}/conversations/`) &&
          (init?.method ?? "GET") === "GET"
        ) {
          detailFetch();
          return jsonResponse([]);
        }
        return undefined;
      },
    );
    const transport = new AevatarAssistantTransport();

    const conversations = await transport.listConversations();

    expect(conversations).toHaveLength(1);
    expect(conversations[0]?.id).toBe(CONVERSATION_ID);
    expect(conversations[0]?.title).toBe("Summarize this week's merged PRs");
    expect(conversations[0]?.message_count).toBe(4);
    expect(conversations[0]?.llm_model).toBe("gpt-5.5");
    expect(conversations[0]?.last_message_at).toBe("2026-07-17T03:05:00+00:00");
    expect(detailFetch).not.toHaveBeenCalled();
  });

  it("keeps a streaming conversation's live title over stale index metadata", async () => {
    stubFetch(routeCreate, routeStream(OBSERVED_FRAMES));
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();
    // Start a turn but don't await it, so the conversation is mid-flight.
    const inflight = collectTurn(transport, "Draft the launch note");

    // A concurrent list refresh must not overwrite the active conversation.
    const list = await transport.listConversations();

    expect(list.find((c) => c.id === CONVERSATION_ID)?.title).toBe(
      "Draft the launch note",
    );
    await inflight;
  });

  it("rejects a concurrent send while a turn is active", async () => {
    stubFetch(routeCreate, routeStream(OBSERVED_FRAMES));
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    const first = collectTurn(transport, "First message");
    expect(() => {
      transport.sendMessage(CONVERSATION_ID, "Second message", () => {});
    }).toThrow(AssistantTurnActiveError);
    await first;
  });

  it("maps RUN_ERROR to a failed turn", async () => {
    stubFetch(
      routeCreate,
      routeStream([
        { type: "RUN_STARTED", turnId: TURN_ID },
        {
          type: "RUN_ERROR",
          runError: { code: "upstream_timeout", message: "Model timed out" },
        },
      ]),
    );
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    const events = await collectTurn(transport, "Hello");

    const terminal = events[events.length - 1];
    expect(
      terminal?.event === "turn.completed" && {
        status: terminal.status,
        error: terminal.error,
      },
    ).toEqual({
      status: "failed",
      error: { code: "upstream_timeout", message: "Model timed out" },
    });
  });

  it("fails the turn when the stream endpoint rejects the request", async () => {
    stubFetch(routeCreate, (url, init) =>
      url.endsWith("/stream") && init?.method === "POST"
        ? jsonResponse({ error: "turn_active" }, 409)
        : undefined,
    );
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    const events = await collectTurn(transport, "Hello");

    const terminal = events[events.length - 1];
    expect(terminal?.event === "turn.completed" && terminal.status).toBe(
      "failed",
    );
    expect(terminal?.event === "turn.completed" && terminal.error?.code).toBe(
      "turn_active",
    );
  });

  it("surfaces the pre-stream error envelope instead of a bare status", async () => {
    // Errors before the SSE stream starts are a JSON `{code, message}`
    // envelope (Chat History contract) — the turn error must carry it.
    stubFetch(routeCreate, (url, init) =>
      url.endsWith("/stream") && init?.method === "POST"
        ? jsonResponse(
            { code: "UPSTREAM_TIMEOUT", message: "Aevatar timed out." },
            502,
          )
        : undefined,
    );
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    const events = await collectTurn(transport, "Hello");

    const terminal = events[events.length - 1];
    const error =
      terminal?.event === "turn.completed" ? terminal.error : undefined;
    expect(error?.code).toBe("UPSTREAM_TIMEOUT");
    expect(error?.message).toBe("Aevatar timed out.");
  });

  it("gives a stream 401 an auth-specific message, not a bare status", async () => {
    stubFetch(routeCreate, (url, init) =>
      url.endsWith("/stream") && init?.method === "POST"
        ? jsonResponse({ error: "unauthorized" }, 401)
        : undefined,
    );
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    const events = await collectTurn(transport, "Hello");

    const terminal = events[events.length - 1];
    const error =
      terminal?.event === "turn.completed" ? terminal.error : undefined;
    expect(error?.code).toBe("unauthorized");
    expect(error?.message).toContain("still signed in");
  });

  it("reports EOF without a terminal frame as a truncated run, keeping partial text", async () => {
    // Idle-killed or dropped streams end mid-run with no RUN_FINISHED /
    // RUN_ERROR. That is not a success (the reference client marks it
    // "closed"); the partial text must still settle into the transcript.
    stubFetch(
      routeCreate,
      routeStream([OBSERVED_FRAMES[0], OBSERVED_FRAMES[1], OBSERVED_FRAMES[2]]),
    );
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    const events = await collectTurn(transport, "Hello");

    const terminal = events[events.length - 1];
    expect(terminal?.event === "turn.completed" && terminal.status).toBe(
      "failed",
    );
    expect(terminal?.event === "turn.completed" && terminal.error?.code).toBe(
      "stream_closed",
    );
    const history = await transport.getHistory(CONVERSATION_ID);
    expect(history.messages[1]?.blocks).toEqual([
      { type: "text", block_id: "m-1-text", text: "Hello, " },
    ]);
  });

  it("fails duplicate terminal frames delivered in separate chunks", async () => {
    stubFetch(routeCreate, (url, init) =>
      url.endsWith("/stream") && init?.method === "POST"
        ? chunkedSseResponse([
            [
              { type: "RUN_STARTED", turnId: TURN_ID },
              { type: "RUN_FINISHED" },
            ],
            [
              {
                type: "RUN_ERROR",
                runError: { code: "late_error", message: "Too late" },
              },
            ],
          ])
        : undefined,
    );
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    const events = await collectTurn(transport, "Hello");

    expect(events.at(-1)).toMatchObject({
      event: "turn.completed",
      status: "failed",
      error: { code: "stream_protocol_error" },
    });
  });

  it("flushes a final RUN_FINISHED frame that has no trailing blank line", async () => {
    const terminatedFrames = OBSERVED_FRAMES.slice(0, -1)
      .map((frame) => `data: ${JSON.stringify(frame)}\n\n`)
      .join("");
    // The capture ends right after the last data line — no blank line.
    const body = `${terminatedFrames}data: ${JSON.stringify({ type: "RUN_FINISHED" })}`;
    stubFetch(routeCreate, (url, init) =>
      url.endsWith("/stream") && init?.method === "POST"
        ? new Response(body, {
            status: 200,
            headers: { "Content-Type": "text/event-stream" },
          })
        : undefined,
    );
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    const events = await collectTurn(transport, "Hello");

    const terminal = events[events.length - 1];
    expect(terminal?.event === "turn.completed" && terminal.status).toBe(
      "completed",
    );
  });

  it("settles open blocks and the turn on cancel", async () => {
    // A stream that emits the message start then stays open forever.
    const openStream = new ReadableStream<Uint8Array>({
      start(controller) {
        const encoder = new TextEncoder();
        controller.enqueue(
          encoder.encode(
            [
              `data: ${JSON.stringify(OBSERVED_FRAMES[0])}\n\n`,
              `data: ${JSON.stringify(OBSERVED_FRAMES[1])}\n\n`,
              `data: ${JSON.stringify(OBSERVED_FRAMES[2])}\n\n`,
            ].join(""),
          ),
        );
      },
    });
    stubFetch(routeCreate, (url, init) =>
      url.endsWith("/stream") && init?.method === "POST"
        ? new Response(openStream, {
            status: 200,
            headers: { "Content-Type": "text/event-stream" },
          })
        : undefined,
    );
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    const events: TurnEvent[] = [];
    const done = new Promise<void>((resolve) => {
      const handle = transport.sendMessage(
        CONVERSATION_ID,
        "Hello",
        (event) => {
          events.push(event);
          if (event.event === "turn.completed") resolve();
          if (event.event === "block.delta") handle.cancel();
        },
      );
    });
    await done;

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
    const terminal = events[events.length - 1];
    expect(terminal?.event === "turn.completed" && terminal.status).toBe(
      "cancelled",
    );
    // Cancel-flow leaves the partial text in the transcript.
    const history = await transport.getHistory(CONVERSATION_ID);
    expect(history.messages[1]?.blocks).toEqual([
      { type: "text", block_id: "m-1-text", text: "Hello, " },
    ]);
  });

  it("fires a best-effort server-side stop carrying the turn identity on cancel", async () => {
    const openStream = new ReadableStream<Uint8Array>({
      start(controller) {
        const encoder = new TextEncoder();
        controller.enqueue(
          encoder.encode(
            [
              `data: ${JSON.stringify(OBSERVED_FRAMES[0])}\n\n`,
              `data: ${JSON.stringify(OBSERVED_FRAMES[1])}\n\n`,
              `data: ${JSON.stringify(OBSERVED_FRAMES[2])}\n\n`,
            ].join(""),
          ),
        );
      },
    });
    const stopBodies: Array<Record<string, unknown>> = [];
    let streamClientRequestId: string | undefined;
    stubFetch(
      routeCreate,
      (url, init) => {
        if (
          url === `${ASSISTANT_BASE}/conversations/${CONVERSATION_ID}/stop` &&
          init?.method === "POST"
        ) {
          stopBodies.push(
            JSON.parse(String(init.body)) as Record<string, unknown>,
          );
          return jsonResponse({ status: "accepted" }, 202);
        }
        return undefined;
      },
      (url, init) => {
        if (url.endsWith("/stream") && init?.method === "POST") {
          streamClientRequestId = (
            JSON.parse(String(init.body)) as { clientRequestId: string }
          ).clientRequestId;
          return new Response(openStream, {
            status: 200,
            headers: { "Content-Type": "text/event-stream" },
          });
        }
        return undefined;
      },
    );
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    await new Promise<void>((resolve) => {
      const handle = transport.sendMessage(
        CONVERSATION_ID,
        "Hello",
        (event) => {
          if (event.event === "turn.completed") resolve();
          if (event.event === "block.delta") handle.cancel();
        },
      );
    });
    // The stop POST is fire-and-forget; give its microtask a beat to land.
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(stopBodies).toHaveLength(1);
    const stop = stopBodies[0];
    expect(stop?.turnId).toBe(TURN_ID);
    expect(stop?.expectedStateVersion).toBe(0);
    // Fresh control identities: neither reuses the turn's clientRequestId.
    expect(stop?.stopRequestId).toMatch(/^[0-9a-f-]{36}$/);
    expect(stop?.clientRequestId).toMatch(/^[0-9a-f-]{36}$/);
    expect(stop?.stopRequestId).not.toBe(stop?.clientRequestId);
    expect(stop?.clientRequestId).not.toBe(streamClientRequestId);
  });

  it("sends no server-side stop when the turn is never announced", async () => {
    // A stream that never sends RUN_STARTED: there is never a turn identity
    // to address, so no stop can go out. (The reader lingers briefly after
    // the local settle — PRE_START_STOP_WINDOW_MS — in case the announcing
    // frame is still in flight; see the next test for that path.)
    const silentStream = new ReadableStream<Uint8Array>({ start() {} });
    const fetchMock = stubFetch(routeCreate, (url, init) =>
      url.endsWith("/stream") && init?.method === "POST"
        ? new Response(silentStream, {
            status: 200,
            headers: { "Content-Type": "text/event-stream" },
          })
        : undefined,
    );
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    await new Promise<void>((resolve) => {
      const handle = transport.sendMessage(
        CONVERSATION_ID,
        "Hello",
        (event) => {
          if (event.event === "turn.completed") resolve();
        },
      );
      setTimeout(() => handle.cancel(), 0);
    });
    await new Promise((resolve) => setTimeout(resolve, 0));

    const stopCalls = fetchMock.mock.calls.filter(([input]) =>
      String(input).endsWith("/stop"),
    );
    expect(stopCalls).toHaveLength(0);
  });

  it("delivers the deferred stop once RUN_STARTED names the turn after a pre-start cancel", async () => {
    // Aevatar may have accepted the stream even though RUN_STARTED has not
    // reached the browser yet. Cancel must not discard the only chance to
    // learn the turnId: the reader stays alive, and the late RUN_STARTED
    // triggers the stop before the connection drops.
    let streamController:
      | ReadableStreamDefaultController<Uint8Array>
      | undefined;
    const lateStream = new ReadableStream<Uint8Array>({
      start(controller) {
        streamController = controller;
      },
    });
    const stopBodies: Array<Record<string, unknown>> = [];
    stubFetch(
      routeCreate,
      (url, init) => {
        if (url.endsWith("/stop") && init?.method === "POST") {
          stopBodies.push(
            JSON.parse(String(init.body)) as Record<string, unknown>,
          );
          return jsonResponse({ status: "accepted" }, 202);
        }
        return undefined;
      },
      (url, init) =>
        url.endsWith("/stream") && init?.method === "POST"
          ? new Response(lateStream, {
              status: 200,
              headers: { "Content-Type": "text/event-stream" },
            })
          : undefined,
    );
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    const events: TurnEvent[] = [];
    const handle = transport.sendMessage(CONVERSATION_ID, "Hello", (event) =>
      events.push(event),
    );
    await new Promise((resolve) => setTimeout(resolve, 10));
    handle.cancel();

    // The local turn settles immediately; no stop yet (no turnId).
    expect(events[events.length - 1]?.event).toBe("turn.completed");
    expect(stopBodies).toHaveLength(0);

    if (!streamController) throw new Error("stream never started");
    streamController.enqueue(
      new TextEncoder().encode(
        `data: ${JSON.stringify(OBSERVED_FRAMES[0])}\n\n`,
      ),
    );
    await new Promise((resolve) => setTimeout(resolve, 30));

    expect(stopBodies).toHaveLength(1);
    expect(stopBodies[0]?.turnId).toBe(TURN_ID);
    expect(stopBodies[0]?.expectedStateVersion).toBe(0);
  });

  it("fences a follow-up send behind a pre-start cancel until the deferred stop settles", async () => {
    // The stop request cannot exist until RUN_STARTED names the turn, but a
    // follow-up send right after a pre-start cancel must already serialize
    // behind the eventual stop — otherwise it can overtake the fence.
    let streamController:
      | ReadableStreamDefaultController<Uint8Array>
      | undefined;
    const lateStream = new ReadableStream<Uint8Array>({
      start(controller) {
        streamController = controller;
      },
    });
    let stopCalls = 0;
    let streamCalls = 0;
    const mock = vi.fn(
      (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
        const url = String(input);
        if (url === `${ASSISTANT_BASE}/conversations` && init?.method === "POST") {
          return Promise.resolve(
            jsonResponse({ status: "accepted", actorId: CONVERSATION_ID }),
          );
        }
        if (url.endsWith("/stop") && init?.method === "POST") {
          stopCalls += 1;
          return Promise.resolve(jsonResponse({ status: "accepted" }, 202));
        }
        if (url.endsWith("/stream") && init?.method === "POST") {
          streamCalls += 1;
          return Promise.resolve(
            streamCalls === 1
              ? new Response(lateStream, {
                  status: 200,
                  headers: { "Content-Type": "text/event-stream" },
                })
              : sseResponse(OBSERVED_FRAMES),
          );
        }
        return Promise.resolve(
          jsonResponse(
            { error: "not_found", error_code: -1, message: "404" },
            404,
          ),
        );
      },
    );
    vi.stubGlobal("fetch", mock);
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    const handle = transport.sendMessage(CONVERSATION_ID, "First", () => {});
    await new Promise((resolve) => setTimeout(resolve, 10));
    handle.cancel(); // pre-start: no RUN_STARTED yet

    const followUp = new Promise<void>((resolve) => {
      transport.sendMessage(CONVERSATION_ID, "Second", (event) => {
        if (event.event === "turn.completed") resolve();
      });
    });
    // The fence holds while the first stream has not announced its turn.
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(streamCalls).toBe(1);
    expect(stopCalls).toBe(0);

    // RUN_STARTED arrives: the deferred stop fires, the fence lifts, and
    // only then does the follow-up stream go out.
    if (!streamController) throw new Error("stream never started");
    streamController.enqueue(
      new TextEncoder().encode(
        `data: ${JSON.stringify(OBSERVED_FRAMES[0])}\n\n`,
      ),
    );
    await followUp;
    expect(stopCalls).toBe(1);
    expect(streamCalls).toBe(2);
  });

  it("holds the pre-start fence beyond two seconds (full pre-start window)", async () => {
    // Regression (codex round 3): an outer 2s race on awaitPendingStop
    // abandoned the fence before the 5s pre-start window elapsed, letting
    // a follow-up overtake a RUN_STARTED that arrived between seconds 2
    // and 5. The fence must hold for the placeholder's full lifetime.
    let streamController:
      | ReadableStreamDefaultController<Uint8Array>
      | undefined;
    const lateStream = new ReadableStream<Uint8Array>({
      start(controller) {
        streamController = controller;
      },
    });
    let stopCalls = 0;
    let streamCalls = 0;
    const mock = vi.fn(
      (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
        const url = String(input);
        if (url === `${ASSISTANT_BASE}/conversations` && init?.method === "POST") {
          return Promise.resolve(
            jsonResponse({ status: "accepted", actorId: CONVERSATION_ID }),
          );
        }
        if (url.endsWith("/stop") && init?.method === "POST") {
          stopCalls += 1;
          return Promise.resolve(jsonResponse({ status: "accepted" }, 202));
        }
        if (url.endsWith("/stream") && init?.method === "POST") {
          streamCalls += 1;
          return Promise.resolve(
            streamCalls === 1
              ? new Response(lateStream, {
                  status: 200,
                  headers: { "Content-Type": "text/event-stream" },
                })
              : sseResponse(OBSERVED_FRAMES),
          );
        }
        return Promise.resolve(
          jsonResponse(
            { error: "not_found", error_code: -1, message: "404" },
            404,
          ),
        );
      },
    );
    vi.stubGlobal("fetch", mock);
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    const handle = transport.sendMessage(CONVERSATION_ID, "First", () => {});
    await new Promise((resolve) => setTimeout(resolve, 10));
    handle.cancel(); // pre-start cancel installs the fence

    const followUp = new Promise<void>((resolve) => {
      transport.sendMessage(CONVERSATION_ID, "Second", (event) => {
        if (event.event === "turn.completed") resolve();
      });
    });
    // Past the old 2s cliff: the fence must still be holding.
    await new Promise((resolve) => setTimeout(resolve, 2_300));
    expect(streamCalls).toBe(1);

    // RUN_STARTED at ~2.4s (inside the 5s window): stop fires, fence
    // lifts, follow-up proceeds.
    if (!streamController) throw new Error("stream never started");
    streamController.enqueue(
      new TextEncoder().encode(
        `data: ${JSON.stringify(OBSERVED_FRAMES[0])}\n\n`,
      ),
    );
    await followUp;
    expect(stopCalls).toBe(1);
    expect(streamCalls).toBe(2);
  }, 15_000);

  it("keeps the earlier stop fence when a queued follow-up is cancelled", async () => {
    // Regression (codex round 4): cancelling a follow-up that is still
    // QUEUED behind an earlier turn's stop must not install a pre-start
    // placeholder — that would overwrite the earlier fence and let a third
    // send overtake the still-pending stop. A never-dispatched run cancels
    // purely locally.
    const encoder = new TextEncoder();
    const firstStream = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(
          encoder.encode(
            [
              `data: ${JSON.stringify(OBSERVED_FRAMES[0])}\n\n`,
              `data: ${JSON.stringify(OBSERVED_FRAMES[1])}\n\n`,
              `data: ${JSON.stringify(OBSERVED_FRAMES[2])}\n\n`,
            ].join(""),
          ),
        );
      },
    });
    let releaseStop: (() => void) | undefined;
    let stopCalls = 0;
    let streamCalls = 0;
    const streamBodies: string[] = [];
    const mock = vi.fn(
      (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
        const url = String(input);
        if (url === `${ASSISTANT_BASE}/conversations` && init?.method === "POST") {
          return Promise.resolve(
            jsonResponse({ status: "accepted", actorId: CONVERSATION_ID }),
          );
        }
        if (url.endsWith("/stop") && init?.method === "POST") {
          stopCalls += 1;
          return new Promise<Response>((resolve) => {
            releaseStop = () =>
              resolve(jsonResponse({ status: "accepted" }, 202));
          });
        }
        if (url.endsWith("/stream") && init?.method === "POST") {
          streamCalls += 1;
          streamBodies.push(String(init?.body));
          return Promise.resolve(
            streamCalls === 1
              ? new Response(firstStream, {
                  status: 200,
                  headers: { "Content-Type": "text/event-stream" },
                })
              : sseResponse(OBSERVED_FRAMES),
          );
        }
        return Promise.resolve(
          jsonResponse(
            { error: "not_found", error_code: -1, message: "404" },
            404,
          ),
        );
      },
    );
    vi.stubGlobal("fetch", mock);
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    // Turn A: streams, gets its turnId, cancelled → stop A held pending.
    await new Promise<void>((resolve) => {
      const handle = transport.sendMessage(CONVERSATION_ID, "Turn A", (e) => {
        if (e.event === "turn.completed") resolve();
        if (e.event === "block.delta") handle.cancel();
      });
    });
    await new Promise((resolve) => setTimeout(resolve, 10));
    expect(stopCalls).toBe(1);
    expect(releaseStop).toBeDefined();

    // Turn B: queued behind stop A (its fetch never dispatches), then
    // cancelled. Must not touch the fence.
    const handleB = transport.sendMessage(CONVERSATION_ID, "Turn B", () => {});
    await new Promise((resolve) => setTimeout(resolve, 10));
    expect(streamCalls).toBe(1);
    handleB.cancel();
    await new Promise((resolve) => setTimeout(resolve, 10));

    // Turn C: must still be fenced by stop A.
    const completedC = new Promise<void>((resolve) => {
      transport.sendMessage(CONVERSATION_ID, "Turn C", (e) => {
        if (e.event === "turn.completed") resolve();
      });
    });
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(streamCalls).toBe(1);

    releaseStop?.();
    await completedC;
    // B never dispatched; C is the second and only other stream, sent
    // after the fence lifted.
    expect(streamCalls).toBe(2);
    expect(streamBodies[1]).toContain("Turn C");
    expect(stopCalls).toBe(1);
  });

  it("rejects sends while a delete is waiting on the stop fence", async () => {
    // Regression (codex round 5): deleteConversation removed the run
    // synchronously, so a successor send admitted during its fence wait
    // could dispatch a stream before the DELETE and recreate the actor.
    const encoder = new TextEncoder();
    const firstStream = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(
          encoder.encode(
            [
              `data: ${JSON.stringify(OBSERVED_FRAMES[0])}\n\n`,
              `data: ${JSON.stringify(OBSERVED_FRAMES[1])}\n\n`,
              `data: ${JSON.stringify(OBSERVED_FRAMES[2])}\n\n`,
            ].join(""),
          ),
        );
      },
    });
    let releaseStop: (() => void) | undefined;
    let deleteCalls = 0;
    const mock = vi.fn(
      (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
        const url = String(input);
        if (url === `${ASSISTANT_BASE}/conversations` && init?.method === "POST") {
          return Promise.resolve(
            jsonResponse({ status: "accepted", actorId: CONVERSATION_ID }),
          );
        }
        if (url.endsWith("/stop") && init?.method === "POST") {
          return new Promise<Response>((resolve) => {
            releaseStop = () =>
              resolve(jsonResponse({ status: "accepted" }, 202));
          });
        }
        if (url.endsWith("/stream") && init?.method === "POST") {
          return Promise.resolve(
            new Response(firstStream, {
              status: 200,
              headers: { "Content-Type": "text/event-stream" },
            }),
          );
        }
        if (init?.method === "DELETE") {
          deleteCalls += 1;
          return Promise.resolve(jsonResponse({}));
        }
        return Promise.resolve(
          jsonResponse(
            { error: "not_found", error_code: -1, message: "404" },
            404,
          ),
        );
      },
    );
    vi.stubGlobal("fetch", mock);
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    await new Promise<void>((resolve) => {
      const handle = transport.sendMessage(CONVERSATION_ID, "Turn A", (e) => {
        if (e.event === "turn.completed") resolve();
        if (e.event === "block.delta") handle.cancel();
      });
    });
    await new Promise((resolve) => setTimeout(resolve, 10));
    expect(releaseStop).toBeDefined();

    const deleting = transport.deleteConversation(CONVERSATION_ID);
    await new Promise((resolve) => setTimeout(resolve, 10));
    expect(deleteCalls).toBe(0); // still fenced behind the held stop
    expect(() =>
      transport.sendMessage(CONVERSATION_ID, "Sneaky send", () => {}),
    ).toThrow("This conversation is being deleted.");

    releaseStop?.();
    await deleting;
    expect(deleteCalls).toBe(1);
    // After success the tombstone takes over.
    expect(() =>
      transport.sendMessage(CONVERSATION_ID, "After delete", () => {}),
    ).toThrow("Conversation was not found.");
  });

  it("guards against re-entrant sends and deletes from the cancellation callback", async () => {
    // Regression (codex round 7): the delete body ran synchronously before
    // the reservation was installed, so the cancel's synchronous
    // `turn.completed` callback could re-enter the transport and slip a
    // send (or a second DELETE) past both guards.
    const encoder = new TextEncoder();
    const openStream = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(
          encoder.encode(
            [
              `data: ${JSON.stringify(OBSERVED_FRAMES[0])}\n\n`,
              `data: ${JSON.stringify(OBSERVED_FRAMES[1])}\n\n`,
              `data: ${JSON.stringify(OBSERVED_FRAMES[2])}\n\n`,
            ].join(""),
          ),
        );
      },
    });
    let deleteCalls = 0;
    const mock = vi.fn(
      (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
        const url = String(input);
        if (url === `${ASSISTANT_BASE}/conversations` && init?.method === "POST") {
          return Promise.resolve(
            jsonResponse({ status: "accepted", actorId: CONVERSATION_ID }),
          );
        }
        if (url.endsWith("/stop") && init?.method === "POST") {
          return Promise.resolve(jsonResponse({ status: "accepted" }, 202));
        }
        if (url.endsWith("/stream") && init?.method === "POST") {
          return Promise.resolve(
            new Response(openStream, {
              status: 200,
              headers: { "Content-Type": "text/event-stream" },
            }),
          );
        }
        if (init?.method === "DELETE") {
          deleteCalls += 1;
          return Promise.resolve(jsonResponse({}));
        }
        return Promise.resolve(
          jsonResponse(
            { error: "not_found", error_code: -1, message: "404" },
            404,
          ),
        );
      },
    );
    vi.stubGlobal("fetch", mock);
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    let deleteStarted = false;
    let reentrantSendError: string | null = null;
    let reentrantDeleteRan = false;
    const streaming = new Promise<void>((resolve) => {
      transport.sendMessage(CONVERSATION_ID, "Turn A", (event) => {
        if (event.event === "block.delta") resolve();
        if (event.event === "turn.completed" && deleteStarted) {
          // Synchronous re-entry from the cancellation callback.
          try {
            transport.sendMessage(CONVERSATION_ID, "reentrant", () => {});
          } catch (error) {
            reentrantSendError =
              error instanceof Error ? error.message : String(error);
          }
          void transport.deleteConversation(CONVERSATION_ID);
          reentrantDeleteRan = true;
        }
      });
    });
    await streaming;

    deleteStarted = true;
    await transport.deleteConversation(CONVERSATION_ID);

    expect(reentrantDeleteRan).toBe(true);
    expect(reentrantSendError).toBe("This conversation is being deleted.");
    // The re-entrant delete coalesced: exactly one DELETE on the wire.
    expect(deleteCalls).toBe(1);
  });

  it("aborts a hung DELETE at its own deadline and stays retryable", async () => {
    // Regression (codex rounds 7-8): the delete carried no deadline, so an
    // accepted-but-unanswered DELETE pinned the reservation forever and
    // locked the conversation. Signal-driven: the mock only rejects when
    // the request's OWN AbortSignal fires, so this fails if the deadline
    // controller, timer, or signal pass-through is ever removed.
    vi.useFakeTimers();
    try {
      let deleteCalls = 0;
      const mock = vi.fn(
        (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
          const url = String(input);
          if (
            url === `${ASSISTANT_BASE}/conversations` &&
            init?.method === "POST"
          ) {
            return Promise.resolve(
              jsonResponse({ status: "accepted", actorId: CONVERSATION_ID }),
            );
          }
          if (init?.method === "DELETE") {
            deleteCalls += 1;
            if (deleteCalls === 1) {
              return new Promise<Response>((_, reject) => {
                init.signal?.addEventListener("abort", () =>
                  reject(new DOMException("aborted", "AbortError")),
                );
              });
            }
            return Promise.resolve(jsonResponse({}));
          }
          return Promise.resolve(
            jsonResponse(
              { error: "not_found", error_code: -1, message: "404" },
              404,
            ),
          );
        },
      );
      vi.stubGlobal("fetch", mock);
      const transport = new AevatarAssistantTransport();
      await transport.createConversation();

      const firstOutcome = transport
        .deleteConversation(CONVERSATION_ID)
        .then(
          () => "resolved",
          () => "rejected",
        );
      const secondOutcome = transport
        .deleteConversation(CONVERSATION_ID)
        .then(
          () => "resolved",
          () => "rejected",
        );

      // Just before the deadline: still one DELETE, still reserved.
      await vi.advanceTimersByTimeAsync(14_000);
      expect(deleteCalls).toBe(1);
      expect(() =>
        transport.sendMessage(CONVERSATION_ID, "too early", () => {}),
      ).toThrow("This conversation is being deleted.");

      // Crossing the deadline aborts the request; both coalesced callers
      // reject and the reservation lifts.
      await vi.advanceTimersByTimeAsync(1_100);
      expect(await firstOutcome).toBe("rejected");
      expect(await secondOutcome).toBe("rejected");

      // Not tombstoned: the retry delete goes through.
      await transport.deleteConversation(CONVERSATION_ID);
      expect(deleteCalls).toBe(2);
      expect(() =>
        transport.sendMessage(CONVERSATION_ID, "after delete", () => {}),
      ).toThrow("Conversation was not found.");
    } finally {
      vi.useRealTimers();
    }
  });

  it("normalizes numeric index timestamps so multi-row lists sort and sends work", async () => {
    // Live-stack repro: the chat-history index carries epoch-ms NUMBERS;
    // with 2+ conversations the sidebar sort called localeCompare on a
    // number and every send died with "Message not sent". (A one-row list
    // never invokes the comparator, which is why single-conversation
    // smoke tests missed it.)
    stubFetch(
      routeCreate,
      (url, init) =>
        url === `${ASSISTANT_BASE}/conversations` &&
        (init?.method ?? "GET") === "GET"
          ? jsonResponse({
              conversations: [
                {
                  id: "conv-old",
                  title: "Older chat",
                  createdAt: 1784192889074,
                  updatedAt: 1784192899074,
                  messageCount: 2,
                },
                {
                  id: "conv-new",
                  title: "Newer chat",
                  createdAt: 1784192989074,
                  updatedAt: 1784192999074,
                  messageCount: 4,
                },
              ],
            })
          : undefined,
      routeStream(OBSERVED_FRAMES),
    );
    const transport = new AevatarAssistantTransport();

    const list = await transport.listConversations();
    expect(list.map((c) => c.id)).toEqual(["conv-new", "conv-old"]);
    for (const conversation of list) {
      expect(typeof conversation.last_message_at).toBe("string");
      expect(typeof conversation.created_at).toBe("string");
    }

    // A send with multiple rows in the mirror must still complete.
    await transport.createConversation();
    const events = await collectTurn(transport, "Does sending still work?");
    expect(events[events.length - 1]?.event).toBe("turn.completed");
  });

  it("coalesces concurrent deletes onto one in-flight operation", async () => {
    // Regression (codex round 6): a Set-style reservation let an
    // overlapping delete clear the flag while the other DELETE was still
    // in flight, re-admitting sends. Concurrent deletes must share one
    // operation — one DELETE on the wire, both callers settle together.
    let releaseDelete: (() => void) | undefined;
    let deleteCalls = 0;
    const mock = vi.fn(
      (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
        const url = String(input);
        if (url === `${ASSISTANT_BASE}/conversations` && init?.method === "POST") {
          return Promise.resolve(
            jsonResponse({ status: "accepted", actorId: CONVERSATION_ID }),
          );
        }
        if (init?.method === "DELETE") {
          deleteCalls += 1;
          return new Promise<Response>((resolve) => {
            releaseDelete = () => resolve(jsonResponse({}));
          });
        }
        return Promise.resolve(
          jsonResponse(
            { error: "not_found", error_code: -1, message: "404" },
            404,
          ),
        );
      },
    );
    vi.stubGlobal("fetch", mock);
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    const first = transport.deleteConversation(CONVERSATION_ID);
    const second = transport.deleteConversation(CONVERSATION_ID);
    await new Promise((resolve) => setTimeout(resolve, 10));
    expect(deleteCalls).toBe(1);
    // While the shared delete is in flight, sends stay rejected.
    expect(() =>
      transport.sendMessage(CONVERSATION_ID, "Sneaky send", () => {}),
    ).toThrow("This conversation is being deleted.");

    releaseDelete?.();
    await Promise.all([first, second]);
    expect(deleteCalls).toBe(1);
    expect(() =>
      transport.sendMessage(CONVERSATION_ID, "After delete", () => {}),
    ).toThrow("Conversation was not found.");
  });

  it("holds an approval decision behind the in-flight stop fence", async () => {
    // Regression (codex round 5): decideApproval dispatched /approve
    // without awaiting the conversation's pending stop, so the approval
    // continuation could overtake a cancelled turn's fence upstream.
    const encoder = new TextEncoder();
    const secondStream = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(
          encoder.encode(
            [
              `data: ${JSON.stringify({ type: "RUN_STARTED", turnId: "turn-2", actorId: CONVERSATION_ID })}\n\n`,
              `data: ${JSON.stringify(OBSERVED_FRAMES[1])}\n\n`,
              `data: ${JSON.stringify(OBSERVED_FRAMES[2])}\n\n`,
            ].join(""),
          ),
        );
      },
    });
    let releaseStop: (() => void) | undefined;
    let approveCalls = 0;
    let streamCalls = 0;
    const mock = vi.fn(
      (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
        const url = String(input);
        if (url === `${ASSISTANT_BASE}/conversations` && init?.method === "POST") {
          return Promise.resolve(
            jsonResponse({ status: "accepted", actorId: CONVERSATION_ID }),
          );
        }
        if (url.endsWith("/stop") && init?.method === "POST") {
          return new Promise<Response>((resolve) => {
            releaseStop = () =>
              resolve(jsonResponse({ status: "accepted" }, 202));
          });
        }
        if (url.endsWith("/approve") && init?.method === "POST") {
          approveCalls += 1;
          return Promise.resolve(sseResponse(OBSERVED_FRAMES));
        }
        if (url.endsWith("/stream") && init?.method === "POST") {
          streamCalls += 1;
          return Promise.resolve(
            streamCalls === 1
              ? sseResponse([
                  { type: "RUN_STARTED", turnId: TURN_ID },
                  {
                    type: "TOOL_APPROVAL_REQUEST",
                    toolApprovalRequest: {
                      requestId: "req-fence",
                      toolName: "lark_post",
                      message: "Post the digest.",
                    },
                  },
                ])
              : new Response(secondStream, {
                  status: 200,
                  headers: { "Content-Type": "text/event-stream" },
                }),
          );
        }
        return Promise.resolve(
          jsonResponse(
            { error: "not_found", error_code: -1, message: "404" },
            404,
          ),
        );
      },
    );
    vi.stubGlobal("fetch", mock);
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    // Turn 1 parks an actionable approval card at EOF.
    const turn1 = await collectTurn(transport, "Post the digest");
    const card = turn1.find(
      (event) =>
        event.event === "block.started" &&
        event.block.type === "approval_card",
    );
    if (!card || card.event !== "block.started") {
      throw new Error("approval card never appeared");
    }

    // Turn 2 streams, gets cancelled mid-delta -> stop held pending.
    await new Promise<void>((resolve) => {
      const handle = transport.sendMessage(CONVERSATION_ID, "Turn 2", (e) => {
        if (e.event === "turn.completed") resolve();
        if (e.event === "block.delta") handle.cancel();
      });
    });
    await new Promise((resolve) => setTimeout(resolve, 10));
    expect(releaseStop).toBeDefined();

    // Deciding the old card must wait for turn 2's stop fence.
    const deciding = transport.decideApproval(
      CONVERSATION_ID,
      card.block.block_id,
      true,
    );
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(approveCalls).toBe(0);

    releaseStop?.();
    await deciding;
    expect(approveCalls).toBe(1);
  });

  it("serializes a follow-up send behind the in-flight stop", async () => {
    // The stop fence must commit upstream before the next :stream goes out;
    // otherwise the follow-up can arrive first and fail with
    // ACTIVE_TURN_REQUIRES_STEERING. Hold the stop 202 pending and assert
    // the second send waits for it.
    const encoder = new TextEncoder();
    const firstStream = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(
          encoder.encode(
            [
              `data: ${JSON.stringify(OBSERVED_FRAMES[0])}\n\n`,
              `data: ${JSON.stringify(OBSERVED_FRAMES[1])}\n\n`,
              `data: ${JSON.stringify(OBSERVED_FRAMES[2])}\n\n`,
            ].join(""),
          ),
        );
      },
    });
    let releaseStop: (() => void) | undefined;
    let streamCalls = 0;
    const mock = vi.fn(
      (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
        const url = String(input);
        if (url === `${ASSISTANT_BASE}/conversations` && init?.method === "POST") {
          return Promise.resolve(
            jsonResponse({ status: "accepted", actorId: CONVERSATION_ID }),
          );
        }
        if (url.endsWith("/stop") && init?.method === "POST") {
          return new Promise<Response>((resolve) => {
            releaseStop = () =>
              resolve(jsonResponse({ status: "accepted" }, 202));
          });
        }
        if (url.endsWith("/stream") && init?.method === "POST") {
          streamCalls += 1;
          return Promise.resolve(
            streamCalls === 1
              ? new Response(firstStream, {
                  status: 200,
                  headers: { "Content-Type": "text/event-stream" },
                })
              : sseResponse(OBSERVED_FRAMES),
          );
        }
        return Promise.resolve(
          jsonResponse(
            { error: "not_found", error_code: -1, message: "404" },
            404,
          ),
        );
      },
    );
    vi.stubGlobal("fetch", mock);
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    await new Promise<void>((resolve) => {
      const handle = transport.sendMessage(
        CONVERSATION_ID,
        "First turn",
        (event) => {
          if (event.event === "turn.completed") resolve();
          if (event.event === "block.delta") handle.cancel();
        },
      );
    });
    await new Promise((resolve) => setTimeout(resolve, 10));
    expect(releaseStop).toBeDefined();
    expect(streamCalls).toBe(1);

    // Follow-up send: must NOT reach /stream while the stop is pending.
    const followUp = new Promise<void>((resolve) => {
      transport.sendMessage(CONVERSATION_ID, "Second turn", (event) => {
        if (event.event === "turn.completed") resolve();
      });
    });
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(streamCalls).toBe(1);

    releaseStop?.();
    await followUp;
    expect(streamCalls).toBe(2);
  });

  it("keeps the local cancel settled when the server-side stop fails", async () => {
    const openStream = new ReadableStream<Uint8Array>({
      start(controller) {
        const encoder = new TextEncoder();
        controller.enqueue(
          encoder.encode(
            [
              `data: ${JSON.stringify(OBSERVED_FRAMES[0])}\n\n`,
              `data: ${JSON.stringify(OBSERVED_FRAMES[1])}\n\n`,
              `data: ${JSON.stringify(OBSERVED_FRAMES[2])}\n\n`,
            ].join(""),
          ),
        );
      },
    });
    stubFetch(
      routeCreate,
      (url, init) =>
        url.endsWith("/stop") && init?.method === "POST"
          ? jsonResponse(
              { error: "internal", error_code: 1006, message: "boom" },
              500,
            )
          : undefined,
      (url, init) =>
        url.endsWith("/stream") && init?.method === "POST"
          ? new Response(openStream, {
              status: 200,
              headers: { "Content-Type": "text/event-stream" },
            })
          : undefined,
    );
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    const events: TurnEvent[] = [];
    await new Promise<void>((resolve) => {
      const handle = transport.sendMessage(
        CONVERSATION_ID,
        "Hello",
        (event) => {
          events.push(event);
          if (event.event === "turn.completed") resolve();
          if (event.event === "block.delta") handle.cancel();
        },
      );
    });
    await new Promise((resolve) => setTimeout(resolve, 0));

    const terminal = events[events.length - 1];
    expect(terminal?.event === "turn.completed" && terminal.status).toBe(
      "cancelled",
    );
  });

  it("throws when deciding an approval for an unknown block", async () => {
    stubFetch(routeHistory([]));
    const transport = new AevatarAssistantTransport();
    await transport.getHistory(CONVERSATION_ID);

    await expect(
      transport.decideApproval(CONVERSATION_ID, "missing-block", true),
    ).rejects.toThrow("Approval request was not found.");
  });

  it("throws when sending to an unknown conversation", () => {
    stubFetch();
    const transport = new AevatarAssistantTransport();
    expect(() => {
      transport.sendMessage("unknown", "Hello", () => {});
    }).toThrow("Conversation was not found.");
  });

  it("never puts a scope id on the wire, even with no user in the store", async () => {
    // The server derives the aevatar scope from the verified session, so the
    // transport must not depend on the client-side user at all (PRD
    // decision 4). Regression guard against reintroducing a scope segment.
    useAuthStore.getState().setUser(null);
    const fetchMock = stubFetch(routeCreate, routeStream(OBSERVED_FRAMES));
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    const events = await collectTurn(transport, "Hello there");

    const terminal = events[events.length - 1];
    expect(terminal?.event === "turn.completed" && terminal.status).toBe(
      "completed",
    );
    for (const [input] of fetchMock.mock.calls) {
      expect(String(input)).not.toContain(USER_ID);
      expect(String(input)).not.toContain("/scopes/");
      expect(String(input)).not.toContain("/proxy/");
    }
  });

  it("deletes a conversation upstream and drops it from the local list", async () => {
    const routeIndex: FetchRoute = (url, init) =>
      url === `${ASSISTANT_BASE}/conversations` &&
      (init?.method ?? "GET") === "GET"
        ? jsonResponse({ conversations: [] })
        : undefined;
    const routeDelete: FetchRoute = (url, init) =>
      url === `${ASSISTANT_BASE}/conversations/${CONVERSATION_ID}` &&
      init?.method === "DELETE"
        ? jsonResponse({})
        : undefined;
    const fetchMock = stubFetch(routeCreate, routeIndex, routeDelete);
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();
    expect(await transport.listConversations()).toHaveLength(1);

    await transport.deleteConversation(CONVERSATION_ID);

    const deleteCall = fetchMock.mock.calls.find(
      ([, init]) => (init as RequestInit | undefined)?.method === "DELETE",
    );
    expect(String(deleteCall?.[0])).toBe(
      `${ASSISTANT_BASE}/conversations/${CONVERSATION_ID}`,
    );
    expect(await transport.listConversations()).toHaveLength(0);
  });

  it("keeps the conversation listed when the upstream delete fails", async () => {
    const routeIndex: FetchRoute = (url, init) =>
      url === `${ASSISTANT_BASE}/conversations` &&
      (init?.method ?? "GET") === "GET"
        ? jsonResponse({ conversations: [] })
        : undefined;
    const routeDeleteFailure: FetchRoute = (url, init) =>
      url === `${ASSISTANT_BASE}/conversations/${CONVERSATION_ID}` &&
      init?.method === "DELETE"
        ? jsonResponse(
            { error: "bad_gateway", error_code: 8002, message: "upstream" },
            502,
          )
        : undefined;
    stubFetch(routeCreate, routeIndex, routeDeleteFailure);
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    await expect(
      transport.deleteConversation(CONVERSATION_ID),
    ).rejects.toThrow();

    // Failed deletes must stay visible and retryable, not vanish locally
    // while the server still has the conversation.
    expect(await transport.listConversations()).toHaveLength(1);
  });

  it("cancels a streaming turn when its conversation is deleted", async () => {
    // A stream that emits the message start then stays open forever, like
    // the cancel test above — deleting mid-turn must settle the turn, not
    // leave the composer waiting on a conversation that no longer exists.
    const openStream = new ReadableStream<Uint8Array>({
      start(controller) {
        const encoder = new TextEncoder();
        controller.enqueue(
          encoder.encode(
            [
              `data: ${JSON.stringify(OBSERVED_FRAMES[0])}\n\n`,
              `data: ${JSON.stringify(OBSERVED_FRAMES[1])}\n\n`,
              `data: ${JSON.stringify(OBSERVED_FRAMES[2])}\n\n`,
            ].join(""),
          ),
        );
      },
    });
    const routeDelete: FetchRoute = (url, init) =>
      url === `${ASSISTANT_BASE}/conversations/${CONVERSATION_ID}` &&
      init?.method === "DELETE"
        ? jsonResponse({})
        : undefined;
    stubFetch(routeCreate, routeDelete, (url, init) =>
      url.endsWith("/stream") && init?.method === "POST"
        ? new Response(openStream, {
            status: 200,
            headers: { "Content-Type": "text/event-stream" },
          })
        : undefined,
    );
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    const events: TurnEvent[] = [];
    let deletion: Promise<void> | null = null;
    const done = new Promise<void>((resolve) => {
      transport.sendMessage(CONVERSATION_ID, "Hello", (event) => {
        events.push(event);
        if (event.event === "turn.completed") resolve();
        if (event.event === "block.delta") {
          deletion = transport.deleteConversation(CONVERSATION_ID);
        }
      });
    });
    await done;
    await deletion;

    const terminal = events[events.length - 1];
    expect(terminal?.event === "turn.completed" && terminal.status).toBe(
      "cancelled",
    );
    expect(
      transport.sendMessage.bind(transport, CONVERSATION_ID, "again", () => {}),
    ).toThrow("Conversation was not found.");
  });
});

// The fixtures under __fixtures__/ are verbatim wire captures taken against
// production aevatar through the NyxID proxy on 2026-07-16 (scratch
// conversation, deleted after capture). These tests replay the REAL bytes —
// if aevatar changes its SSE framing or frame shapes, refresh the captures
// and these tests tell you exactly what broke.
describe("captured production wire shapes", () => {
  const CAPTURED_ANSWER = "Blue is a color.  \nGreen is a color.";
  const CAPTURED_MESSAGE_ID = "31c82249c61e42239075795cfa9306d9";

  it("replays the captured SSE stream byte-for-byte in awkward chunks", async () => {
    // The capture predates Aevatar's required server-owned turn identity.
    // Preserve every captured byte except for adding that contract field.
    const currentContractStream = capturedStream.replace(
      `"actorId":"nyxid-chat-4a1e60ebd1fd44f192bf4bb90e1812ae"`,
      `"turnId":"${TURN_ID}","actorId":"nyxid-chat-4a1e60ebd1fd44f192bf4bb90e1812ae"`,
    );
    const bytes = new TextEncoder().encode(currentContractStream);
    // 7-byte chunks split `data:` prefixes, JSON payloads, and the \n\n
    // frame boundary mid-sequence — the incremental parser must not care.
    const CHUNK = 7;
    const trickle = new ReadableStream<Uint8Array>({
      start(controller) {
        for (let i = 0; i < bytes.length; i += CHUNK) {
          controller.enqueue(bytes.slice(i, i + CHUNK));
        }
        controller.close();
      },
    });
    stubFetch(routeCreate, (url, init) =>
      url.endsWith("/stream") && init?.method === "POST"
        ? new Response(trickle, {
            status: 200,
            headers: { "Content-Type": "text/event-stream" },
          })
        : undefined,
    );
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    const events = await collectTurn(
      transport,
      "Name two colors. Answer in two short sentences.",
    );

    expect(events.map((event) => event.event)).toEqual([
      "turn.status",
      "message.started",
      "block.started",
      "block.delta",
      "block.completed",
      "message.completed",
      "turn.completed",
    ]);
    const completed = events.find((event) => event.event === "block.completed");
    expect(completed?.event === "block.completed" && completed.block).toEqual({
      type: "text",
      block_id: `${CAPTURED_MESSAGE_ID}-text`,
      text: CAPTURED_ANSWER,
    });
    const terminal = events[events.length - 1];
    expect(terminal?.event === "turn.completed" && terminal.status).toBe(
      "completed",
    );
  });

  it("maps the captured history payload into transcript messages", async () => {
    stubFetch(routeHistory(capturedHistory));
    const transport = new AevatarAssistantTransport();

    const history = await transport.getHistory(CONVERSATION_ID);

    expect(history.messages).toHaveLength(2);
    expect(history.messages[0]?.role).toBe("user");
    expect(history.messages[0]?.blocks[0]).toMatchObject({
      type: "text",
      text: "Name two colors. Answer in two short sentences.",
    });
    expect(history.messages[1]?.role).toBe("assistant");
    expect(history.messages[1]?.blocks[0]).toMatchObject({
      type: "text",
      text: CAPTURED_ANSWER,
    });
    expect(history.conversation.title).toBe(
      "Name two colors. Answer in two short sen",
    );
  });

  // Aevatar PR #2923 wrapped the transcript in `{messages, stateVersion}`.
  // Both shapes must map to the identical transcript, and the wrapper must
  // not be confused with a body that merely happens to be an object.
  it("maps the PR #2923 wrapped transcript identically to the legacy array", async () => {
    stubFetch(
      routeHistory({ messages: capturedHistory, stateVersion: 7 }),
    );
    const transport = new AevatarAssistantTransport();

    const wrapped = await transport.getHistory(CONVERSATION_ID);

    stubFetch(routeHistory(capturedHistory));
    const legacy = await new AevatarAssistantTransport().getHistory(
      CONVERSATION_ID,
    );
    expect(wrapped.messages).toEqual(legacy.messages);
    expect(wrapped.conversation.title).toBe(legacy.conversation.title);
  });

  it.each([
    ["with a stateVersion", { messages: [], stateVersion: 0 }],
    // Acceptance is keyed ONLY on array-valued `messages`. `stateVersion` has
    // zero consumers on the `:stream` transport, so requiring it would turn a
    // field we never read into an outage.
    ["without a stateVersion", { messages: [] }],
  ])(
    "treats an empty wrapped transcript %s as a valid empty conversation",
    async (_label, body) => {
      // The contract's "empty is a real answer" rule (deleted / not yet
      // materialized / zero turns) survives the wrapper — it must not be
      // mistaken for a shape violation.
      stubFetch(routeHistory(body));
      const transport = new AevatarAssistantTransport();

      const history = await transport.getHistory(CONVERSATION_ID);

      expect(history.messages).toEqual([]);
    },
  );

  it.each([
    ["no messages field", {}],
    ["a null messages field", { messages: null }],
    ["a non-array messages field", { messages: { "0": {} } }],
    ["a bare string", "nyxid-chat-f836"],
  ])(
    "surfaces a transcript response with %s instead of rendering it empty",
    async (_label, body) => {
      // The regression this whole change exists for: an unrecognized body
      // must NOT be laundered into "this chat has no messages". The index
      // has already merged the conversation in (mirror present but empty),
      // which is exactly the state the old catch-all fallback dressed up as
      // a successful empty read.
      stubFetch(
        (url, init) =>
          url === `${ASSISTANT_BASE}/conversations` &&
          (init?.method ?? "GET") === "GET"
            ? jsonResponse({
                conversations: [
                  { id: CONVERSATION_ID, title: "Server title" },
                ],
              })
            : undefined,
        routeHistory(body),
      );
      const transport = new AevatarAssistantTransport();
      await transport.listConversations();

      await expect(transport.getHistory(CONVERSATION_ID)).rejects.toThrow(
        /did not match the expected shape/,
      );
    },
  );

  it("sends the exact request shape the aevatar stream endpoint requires", async () => {
    const fetchMock = stubFetch(routeCreate, routeStream(OBSERVED_FRAMES));
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    await collectTurn(transport, "Hello there");

    const streamCall = fetchMock.mock.calls.find(([input]) =>
      String(input).endsWith("/stream"),
    ) as [string, RequestInit] | undefined;
    expect(streamCall).toBeDefined();
    const [url, init] = streamCall ?? ["", {}];
    // NyxID's own route: no scope segment, because the server derives the
    // aevatar scope from the verified session. The endpoint still 415s
    // without the explicit JSON content type.
    expect(url).toBe(
      `${ASSISTANT_BASE}/conversations/${CONVERSATION_ID}/stream`,
    );
    expect(url).not.toContain(USER_ID);
    expect(url).not.toContain("/proxy/");
    expect(init.method).toBe("POST");
    expect(init.credentials).toBe("include");
    expect(init.headers).toMatchObject({
      "Content-Type": "application/json",
      Accept: "text/event-stream",
    });
    const body = JSON.parse(String(init.body)) as {
      prompt: string;
      clientRequestId: string;
      type: string;
      sessionId?: string;
    };
    expect(body.prompt).toBe("Hello there");
    expect(body.clientRequestId).toMatch(
      /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/,
    );
    expect(body.type).toBe("text");
    expect(Object.keys(body).sort()).toEqual([
      "clientRequestId",
      "prompt",
      "type",
    ]);
    expect(body.sessionId).toBeUndefined();
  });

  it("uses a new clientRequestId for each logical turn across reprojection", async () => {
    const fetchMock = stubFetch(
      routeCreate,
      routeStream(OBSERVED_FRAMES),
      (url, init) =>
        url === `${ASSISTANT_BASE}/conversations` &&
        (init?.method ?? "GET") === "GET"
          ? jsonResponse({
              conversations: [
                {
                  id: CONVERSATION_ID,
                  title: "Say hello",
                  updatedAt: "2026-07-17T03:05:00+00:00",
                  messageCount: 1,
                },
              ],
            })
          : undefined,
      routeHistory([
        {
          id: "t1:user",
          role: "user",
          content: "First turn",
          timestamp: 1784192889074,
        },
        {
          id: "t1:assistant",
          role: "assistant",
          content: "Hello, hope your day shines.",
          timestamp: 1784192899074,
        },
      ]),
    );
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    await collectTurn(transport, "First turn");
    // Post-turn reprojection: server history (equal length) replaces the
    // local mirror; the list merge rebuilds the conversation row.
    await transport.getHistory(CONVERSATION_ID);
    await transport.listConversations();
    await collectTurn(transport, "Second turn");

    const clientRequestIds = fetchMock.mock.calls
      .filter(([input]) => String(input).endsWith("/stream"))
      .map(
        ([, init]) =>
          (JSON.parse(String(init?.body)) as { clientRequestId: string })
            .clientRequestId,
      );
    expect(clientRequestIds).toHaveLength(2);
    expect(clientRequestIds[0]).not.toBe(clientRequestIds[1]);
  });

  it("reuses one clientRequestId for an automatic transport retry", async () => {
    let streamAttempts = 0;
    const fetchMock = vi.fn(
      (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
        const url = String(input);
        if (
          url === `${ASSISTANT_BASE}/conversations` &&
          init?.method === "POST"
        ) {
          return Promise.resolve(
            jsonResponse({ status: "accepted", actorId: CONVERSATION_ID }),
          );
        }
        if (url.endsWith("/stream") && init?.method === "POST") {
          streamAttempts += 1;
          if (streamAttempts === 1) {
            return Promise.reject(new TypeError("connection reset"));
          }
          return Promise.resolve(sseResponse(OBSERVED_FRAMES));
        }
        return Promise.resolve(jsonResponse({}, 404));
      },
    );
    vi.stubGlobal("fetch", fetchMock);
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    const events = await collectTurn(transport, "Retry this delivery");

    expect(events.at(-1)).toMatchObject({
      event: "turn.completed",
      turn_id: TURN_ID,
      status: "completed",
    });
    const bodies = fetchMock.mock.calls
      .filter(([input]) => String(input).endsWith("/stream"))
      .map(
        ([, init]) =>
          JSON.parse(String(init?.body)) as {
            prompt: string;
            clientRequestId: string;
            type: string;
            sessionId?: string;
          },
      );
    expect(bodies).toHaveLength(2);
    expect(bodies[0]?.clientRequestId).toBe(bodies[1]?.clientRequestId);
    expect(bodies[0]).toEqual({
      prompt: "Retry this delivery",
      clientRequestId: bodies[0]?.clientRequestId,
      type: "text",
    });
    expect(bodies[1]).toEqual(bodies[0]);
    expect(bodies[0]?.sessionId).toBeUndefined();
  });

  it("retries a successful stream response that has no body", async () => {
    let streamAttempts = 0;
    const fetchMock = stubFetch(routeCreate, (url, init) => {
      if (!url.endsWith("/stream") || init?.method !== "POST") return undefined;
      streamAttempts += 1;
      return streamAttempts === 1
        ? new Response(null, {
            status: 200,
            headers: { "Content-Type": "text/event-stream" },
          })
        : sseResponse(OBSERVED_FRAMES);
    });
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    const events = await collectTurn(transport, "Retry the empty delivery");

    expect(events.at(-1)).toMatchObject({
      event: "turn.completed",
      turn_id: TURN_ID,
      status: "completed",
    });
    const requestIds = fetchMock.mock.calls
      .filter(([input]) => String(input).endsWith("/stream"))
      .map(
        ([, requestInit]) =>
          (JSON.parse(String(requestInit?.body)) as { clientRequestId: string })
            .clientRequestId,
      );
    expect(requestIds).toHaveLength(2);
    expect(requestIds[0]).toBe(requestIds[1]);
  });
});

// The reference client (eanz17/nyxid-chat `protocol.js`) documents the FULL
// live AG-UI vocabulary; these tests pin our adapter's mapping of every frame
// family onto the PRD §3.5 block types the UI renders.
describe("live AG-UI frame taxonomy", () => {
  function blockStarts(events: TurnEvent[]): ContentBlock[] {
    return events
      .filter(
        (event): event is Extract<TurnEvent, { event: "block.started" }> =>
          event.event === "block.started",
      )
      .map((event) => event.block);
  }

  function blockCompletions(events: TurnEvent[]): ContentBlock[] {
    return events
      .filter(
        (event): event is Extract<TurnEvent, { event: "block.completed" }> =>
          event.event === "block.completed",
      )
      .map((event) => event.block);
  }

  function routeApprove(response: () => Response): FetchRoute {
    return (url, init) =>
      url === `${ASSISTANT_BASE}/conversations/${CONVERSATION_ID}/approve` &&
      init?.method === "POST"
        ? response()
        : undefined;
  }

  it("maps TOOL_CALL_START/END onto a run step ledger", async () => {
    stubFetch(
      routeCreate,
      routeStream([
        { type: "RUN_STARTED", turnId: TURN_ID, actorId: CONVERSATION_ID },
        {
          type: "TOOL_CALL_START",
          toolCallStart: { toolCallId: "c1", toolName: "ornn_search_skills" },
        },
        {
          type: "TOOL_CALL_END",
          toolCallEnd: {
            toolCallId: "c1",
            status: "AGENT_TOOL_RECEIPT_STATUS_SUCCESS",
            result: { found: true },
          },
        },
        { type: "RUN_FINISHED" },
      ]),
    );
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    const events = await collectTurn(transport, "Search skills");

    const runStart = blockStarts(events).find((block) => block.type === "run");
    expect(runStart?.type === "run" && runStart.steps).toEqual([
      {
        index: 1,
        status: "active",
        label: "ornn_search_skills",
        meta: "Running",
        service_slug: null,
        artifact_id: null,
        approval_request_id: null,
      },
    ]);
    // §3.7: a patch touching steps carries the complete array.
    const patches = events.filter(
      (event): event is Extract<TurnEvent, { event: "block.updated" }> =>
        event.event === "block.updated",
    );
    expect(patches.length).toBeGreaterThan(0);
    for (const patch of patches) {
      expect(Array.isArray((patch.patch as { steps?: unknown }).steps)).toBe(
        true,
      );
    }
    const runFinal = blockCompletions(events).find(
      (block) => block.type === "run",
    );
    expect(runFinal?.type === "run" && runFinal.state).toBe("completed");
    expect(runFinal?.type === "run" && runFinal.steps[0]).toMatchObject({
      status: "done",
      label: "ornn_search_skills",
    });
    expect(runFinal?.type === "run" && runFinal.steps[0]?.meta).toContain(
      "found",
    );
    const terminal = events[events.length - 1];
    expect(terminal?.event === "turn.completed" && terminal.status).toBe(
      "completed",
    );
  });

  it("keeps policy-denied tool failures ordinary without a connect card", async () => {
    stubFetch(
      routeCreate,
      routeStream([
        { type: "RUN_STARTED", turnId: TURN_ID },
        {
          type: "TOOL_CALL_START",
          toolCallStart: { toolCallId: "c1", toolName: "github_write" },
        },
        {
          type: "TOOL_CALL_END",
          toolCallEnd: {
            toolCallId: "c1",
            status: "AGENT_TOOL_RECEIPT_STATUS_ERROR",
            error: {
              status: 403,
              error: "forbidden",
              error_code: 1002,
              message: "user is not authorized for service api-github",
            },
          },
        },
        { type: "RUN_FINISHED" },
      ]),
    );
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    const events = await collectTurn(transport, "Open a PR");

    expect(
      blockStarts(events).some((block) => block.type === "connect_card"),
    ).toBe(false);
    const runFinal = blockCompletions(events).find(
      (block) => block.type === "run",
    );
    expect(runFinal?.type === "run" && runFinal.state).toBe("failed");
    expect(events.at(-1)).toMatchObject({
      event: "turn.completed",
      status: "completed",
      error: null,
    });
  });

  it("ignores generic and unclassified authorization signals", async () => {
    stubFetch(
      routeCreate,
      routeStream([
        { type: "RUN_STARTED", turnId: TURN_ID },
        {
          type: "AUTHORIZATION_REQUIRED",
          authorizationRequired: { serviceSlug: "api-github" },
        },
        {
          type: "CUSTOM",
          custom: {
            name: "aevatar.authorization.required",
            payload: { serviceSlug: "api-github" },
          },
        },
        {
          type: "CUSTOM",
          custom: {
            name: "nyxid.authorization.required",
            payload: {
              serviceSlug: "api-github",
              reasonCode: "POLICY_DENIED",
              safeMessage: "Policy denied this request.",
            },
          },
        },
        { type: "RUN_FINISHED" },
      ]),
    );
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    const events = await collectTurn(transport, "Summarize PRs");

    expect(
      blockStarts(events).some((block) => block.type === "connect_card"),
    ).toBe(false);
    expect(events.at(-1)).toMatchObject({ status: "completed" });
  });

  it("maps only the typed NyxID blocker and terminal status to blocked", async () => {
    const authorizationFrame = {
      type: "CUSTOM",
      custom: {
        name: "nyxid.authorization.required",
        payload: {
          serviceSlug: "api-github",
          serviceLabel: "GitHub",
          resourceUri: "/repos/private?access_token=do-not-render",
          reasonCode: "NYXID_UNAUTHORIZED",
          safeMessage: "Connect GitHub; token=secret-value is expired.",
          arbitrarySecret: "must-not-be-copied",
        },
      },
    };
    stubFetch(
      routeCreate,
      routeStream([
        { type: "RUN_STARTED", turnId: TURN_ID },
        authorizationFrame,
        authorizationFrame,
        {
          type: "RUN_FINISHED",
          turnId: TURN_ID,
          runFinished: { runId: TURN_ID, status: "blocked" },
        },
      ]),
    );
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    const events = await collectTurn(transport, "Summarize my PRs");

    const cards = blockStarts(events).filter(
      (block) => block.type === "connect_card",
    );
    expect(cards).toHaveLength(1);
    const card = cards[0];
    expect(card?.type === "connect_card" && card.catalog_slug).toBe(
      "api-github",
    );
    expect(card?.type === "connect_card" && card.reason_code).toBe(
      "NYXID_UNAUTHORIZED",
    );
    expect(card?.type === "connect_card" && card.steps[0]?.body).toBe(
      'Connect GitHub; token="[redacted]" is expired.',
    );
    expect(JSON.stringify(events)).not.toContain("do-not-render");
    expect(JSON.stringify(events)).not.toContain("must-not-be-copied");
    const terminal = events[events.length - 1];
    expect(terminal?.event === "turn.completed" && terminal.status).toBe(
      "blocked",
    );
  });

  it("maps a genuinely disconnected service by its canonical slug", async () => {
    stubFetch(
      routeCreate,
      routeStream([
        { type: "RUN_STARTED", turnId: TURN_ID },
        {
          type: "CUSTOM",
          custom: {
            name: "nyxid.authorization.required",
            payload: {
              serviceSlug: "api-lark-bot",
              serviceLabel: "Lark Bot",
              reasonCode: "NYXID_SERVICE_NOT_CONNECTED",
              safeMessage: "Connect your Lark bot first.",
            },
          },
        },
        {
          type: "RUN_FINISHED",
          runFinished: { status: "blocked" },
        },
      ]),
    );
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    const events = await collectTurn(transport, "Post to Lark");

    const card = blockStarts(events).find(
      (block) => block.type === "connect_card",
    );
    expect(card?.type === "connect_card" && card.catalog_slug).toBe(
      "api-lark-bot",
    );
    expect(card?.type === "connect_card" && card.catalog_slug).not.toBe(
      "api-api-lark-bot",
    );
    expect(card?.type === "connect_card" && card.reason_code).toBe(
      "NYXID_SERVICE_NOT_CONNECTED",
    );
  });

  it("starts a new logical turn after a blocked delivery", async () => {
    let delivery = 0;
    const fetchMock = stubFetch(routeCreate, (url, init) => {
      if (!url.endsWith("/stream") || init?.method !== "POST") return undefined;
      delivery += 1;
      return delivery === 1
        ? sseResponse([
            { type: "RUN_STARTED", turnId: TURN_ID },
            {
              type: "CUSTOM",
              custom: {
                name: "nyxid.authorization.required",
                payload: {
                  serviceSlug: "api-github",
                  serviceLabel: "GitHub",
                  reasonCode: "NYXID_UNAUTHORIZED",
                  safeMessage: "Reconnect GitHub to continue.",
                },
              },
            },
            {
              type: "RUN_FINISHED",
              runFinished: { status: "blocked" },
            },
          ])
        : sseResponse([
            { type: "RUN_STARTED", turnId: "turn-server-owned-2" },
            { type: "RUN_FINISHED", runFinished: { status: "completed" } },
          ]);
    });
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    const blocked = await collectTurn(transport, "Read a private repository");
    const completed = await collectTurn(transport, "Continue after reconnect");

    expect(blocked.at(-1)).toMatchObject({ status: "blocked" });
    expect(completed.at(-1)).toMatchObject({
      status: "completed",
      turn_id: "turn-server-owned-2",
    });
    const requestIds = fetchMock.mock.calls
      .filter(([input]) => String(input).endsWith("/stream"))
      .map(
        ([, init]) =>
          (JSON.parse(String(init?.body)) as { clientRequestId: string })
            .clientRequestId,
      );
    expect(requestIds).toHaveLength(2);
    expect(requestIds[0]).not.toBe(requestIds[1]);
  });

  it("parks the turn on TOOL_APPROVAL_REQUEST and keeps the card actionable at EOF", async () => {
    // The upstream may close the idle stream while the human gate is open
    // (PRD §3.4); that is a pause, not a truncated run.
    stubFetch(
      routeCreate,
      routeStream([
        { type: "RUN_STARTED", turnId: TURN_ID },
        {
          type: "TOOL_APPROVAL_REQUEST",
          toolApprovalRequest: {
            requestId: "req-1",
            toolName: "lark_post",
            message: "Post the digest to #eng-updates.",
          },
        },
      ]),
    );
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    const events = await collectTurn(transport, "Post the digest");

    const card = blockStarts(events).find(
      (block) => block.type === "approval_card",
    );
    expect(card?.type === "approval_card" && card.approval_request_id).toBe(
      "req-1",
    );
    expect(card?.type === "approval_card" && card.body).toBe(
      "Post the digest to #eng-updates.",
    );
    expect(
      events.some(
        (event) => event.event === "turn.status" && event.status === "waiting",
      ),
    ).toBe(true);
    const terminal = events[events.length - 1];
    expect(
      terminal?.event === "turn.completed" && {
        status: terminal.status,
        error: terminal.error,
      },
    ).toEqual({ status: "completed", error: null });
    const cardFinal = blockCompletions(events).find(
      (block) => block.type === "approval_card",
    );
    expect(cardFinal?.type === "approval_card" && cardFinal.decision).toBe(
      null,
    );
  });

  it("streams the approve endpoint's SSE continuation as a follow-on turn", async () => {
    const fetchMock = stubFetch(
      routeCreate,
      routeStream([
        { type: "RUN_STARTED", turnId: TURN_ID },
        {
          type: "TOOL_APPROVAL_REQUEST",
          toolApprovalRequest: { requestId: "req-1", toolName: "lark_post" },
        },
      ]),
      routeApprove(() =>
        sseResponse([
          { type: "RUN_STARTED", turnId: TURN_ID },
          {
            type: "TEXT_MESSAGE_START",
            textMessageStart: { messageId: "m-2", role: "assistant" },
          },
          {
            type: "TEXT_MESSAGE_CONTENT",
            textMessageContent: { delta: "Posted to #eng-updates." },
          },
          { type: "TEXT_MESSAGE_END", textMessageEnd: { messageId: "m-2" } },
          { type: "RUN_FINISHED" },
        ]),
      ),
      routeHistory([]),
    );
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();
    const firstTurn = await collectTurn(transport, "Post the digest");
    const lastFirstCursor = firstTurn[firstTurn.length - 1]?.cursor ?? 0;
    const history = await transport.getHistory(CONVERSATION_ID);
    const card = history.messages
      .flatMap((message) => message.blocks)
      .find((block) => block.type === "approval_card");
    expect(card).toBeDefined();

    const events: TurnEvent[] = [];
    const done = new Promise<void>((resolve) => {
      void transport
        .decideApproval(
          CONVERSATION_ID,
          card?.block_id ?? "",
          true,
          (event) => {
            events.push(event);
            if (event.event === "turn.completed") resolve();
          },
        )
        .then((handle) => {
          expect(handle).not.toBeNull();
        });
    });
    await done;

    const approveCall = fetchMock.mock.calls.find(([input]) =>
      String(input).endsWith("/approve"),
    );
    const approveBody = JSON.parse(
      String((approveCall?.[1] as RequestInit | undefined)?.body),
    ) as { requestId: string; approved: boolean; sessionId?: string };
    expect(approveBody.requestId).toBe("req-1");
    expect(approveBody.approved).toBe(true);
    expect(approveBody.sessionId).toBeUndefined();

    // Continuation cursors continue past the prior turn's, so a
    // still-subscribed at-least-once consumer never drops them.
    expect(events[0]?.cursor).toBeGreaterThan(lastFirstCursor);
    const cursors = events.map((event) => event.cursor);
    expect(cursors).toEqual([...cursors].sort((a, b) => a - b));
    const flip = events.find(
      (event): event is Extract<TurnEvent, { event: "block.updated" }> =>
        event.event === "block.updated",
    );
    expect(flip?.patch).toMatchObject({
      decision: "approved",
      decision_channel: "web",
    });
    const text = blockCompletions(events).find(
      (block) => block.type === "text",
    );
    expect(text?.type === "text" && text.text).toBe("Posted to #eng-updates.");
    const terminal = events[events.length - 1];
    expect(terminal?.event === "turn.completed" && terminal.status).toBe(
      "completed",
    );
  });

  it("fails closed when an approval continuation ends without a terminal frame", async () => {
    stubFetch(
      routeCreate,
      routeStream([
        { type: "RUN_STARTED", turnId: TURN_ID },
        {
          type: "TOOL_APPROVAL_REQUEST",
          toolApprovalRequest: { requestId: "req-truncated" },
        },
      ]),
      routeApprove(() =>
        sseResponse([
          { type: "RUN_STARTED", turnId: TURN_ID },
          {
            type: "TEXT_MESSAGE_START",
            textMessageStart: { messageId: "m-truncated", role: "assistant" },
          },
          {
            type: "TEXT_MESSAGE_CONTENT",
            textMessageContent: { delta: "Partial continuation" },
          },
        ]),
      ),
      routeHistory([]),
    );
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();
    await collectTurn(transport, "Run an approved action");
    const history = await transport.getHistory(CONVERSATION_ID);
    const card = history.messages
      .flatMap((message) => message.blocks)
      .find((block) => block.type === "approval_card");

    const events: TurnEvent[] = [];
    await new Promise<void>((resolve) => {
      void transport.decideApproval(
        CONVERSATION_ID,
        card?.block_id ?? "",
        true,
        (event) => {
          events.push(event);
          if (event.event === "turn.completed") resolve();
        },
      );
    });

    expect(events.at(-1)).toMatchObject({
      event: "turn.completed",
      turn_id: TURN_ID,
      status: "failed",
      error: { code: "stream_closed" },
    });
  });

  it("reserves the conversation for the whole approve exchange", async () => {
    // Codex P1: without a reservation, the await on the approve fetch left
    // the conversation looking idle — a concurrent send could slip past the
    // active-turn guard and interleave two streams into one reducer.
    let releaseApprove: () => void = () => {};
    stubFetch(
      routeCreate,
      routeStream([
        { type: "RUN_STARTED", turnId: TURN_ID },
        {
          type: "TOOL_APPROVAL_REQUEST",
          toolApprovalRequest: { requestId: "req-slow" },
        },
      ]),
      routeHistory([]),
    );
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();
    await collectTurn(transport, "Do the thing");
    const history = await transport.getHistory(CONVERSATION_ID);
    const card = history.messages
      .flatMap((message) => message.blocks)
      .find((block) => block.type === "approval_card");

    // Swap fetch for one whose /approve hangs until released.
    const baseFetch = fetch;
    let approveSignal: AbortSignal | null | undefined;
    vi.stubGlobal(
      "fetch",
      vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
        if (String(input).endsWith("/approve")) {
          approveSignal = init?.signal;
          return new Promise<Response>((resolve) => {
            releaseApprove = () => {
              resolve(jsonResponse({ accepted: true }));
            };
          });
        }
        return baseFetch(input, init);
      }),
    );

    const pending = transport.decideApproval(
      CONVERSATION_ID,
      card?.block_id ?? "",
      true,
    );
    // Yield so decideApproval reaches the in-flight fetch.
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(() => {
      transport.sendMessage(CONVERSATION_ID, "concurrent send", () => {});
    }).toThrow(AssistantTurnActiveError);
    // Stop must be able to abort the in-flight approve request.
    expect(approveSignal).toBeDefined();
    releaseApprove();
    await expect(pending).resolves.toBeNull();
  });

  it("marks the pending tool step waiting on the dedicated approval frame", async () => {
    // Codex P2: TOOL_CALL_START directly followed by TOOL_APPROVAL_REQUEST
    // (no toolCallId on the frame) must park the step, not spin forever.
    stubFetch(
      routeCreate,
      routeStream([
        { type: "RUN_STARTED", turnId: TURN_ID },
        {
          type: "TOOL_CALL_START",
          toolCallStart: { toolCallId: "c1", toolName: "lark_post" },
        },
        {
          type: "TOOL_APPROVAL_REQUEST",
          toolApprovalRequest: { requestId: "req-9" },
        },
      ]),
    );
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    const events = await collectTurn(transport, "Post it");

    const runFinal = blockCompletions(events).find(
      (block) => block.type === "run",
    );
    expect(runFinal?.type === "run" && runFinal.steps[0]).toMatchObject({
      status: "waiting",
      approval_request_id: "req-9",
    });
    expect(runFinal?.type === "run" && runFinal.state).toBe(
      "awaiting_approval",
    );
  });

  it("keeps a deleted conversation out of stale index responses", async () => {
    // Codex P2: the Chat History index is eventually consistent; a stale
    // list response must not resurrect a server-accepted delete.
    let listCalls = 0;
    stubFetch(
      routeCreate,
      (url, init) =>
        url === `${ASSISTANT_BASE}/conversations` &&
        (init?.method ?? "GET") === "GET"
          ? ((listCalls += 1),
            jsonResponse({
              conversations:
                listCalls <= 2
                  ? [{ id: CONVERSATION_ID, title: "Stale row" }]
                  : [],
            }))
          : undefined,
      (url, init) =>
        url === `${ASSISTANT_BASE}/conversations/${CONVERSATION_ID}` &&
        init?.method === "DELETE"
          ? jsonResponse({})
          : undefined,
    );
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    await transport.deleteConversation(CONVERSATION_ID);

    // Stale index still returns the row — the tombstone must filter it.
    const list = await transport.listConversations();
    expect(list).toHaveLength(0);
  });

  it("settles immediately when the approve endpoint acks with JSON", async () => {
    stubFetch(
      routeCreate,
      routeStream([
        { type: "RUN_STARTED", turnId: TURN_ID },
        {
          type: "TOOL_APPROVAL_REQUEST",
          toolApprovalRequest: { requestId: "req-2" },
        },
      ]),
      routeApprove(() => jsonResponse({ accepted: true })),
      routeHistory([]),
    );
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();
    await collectTurn(transport, "Do the thing");
    const history = await transport.getHistory(CONVERSATION_ID);
    const card = history.messages
      .flatMap((message) => message.blocks)
      .find((block) => block.type === "approval_card");

    const events: TurnEvent[] = [];
    const handle = await transport.decideApproval(
      CONVERSATION_ID,
      card?.block_id ?? "",
      false,
      (event) => events.push(event),
    );

    // No live continuation — no handle to register (a stale entry would
    // linger in the caller's registry after the turn completed).
    expect(handle).toBeNull();
    const flip = events.find(
      (event): event is Extract<TurnEvent, { event: "block.updated" }> =>
        event.event === "block.updated",
    );
    expect(flip?.patch).toMatchObject({ decision: "denied" });
    const terminal = events[events.length - 1];
    expect(terminal?.event === "turn.completed" && terminal.status).toBe(
      "completed",
    );
  });

  it("settles the parked run ledger when the approval is decided", async () => {
    // The prior turn's ledger froze in awaiting_approval with a waiting
    // step; deciding must flip it (approved → done/completed) so the
    // transient activity line doesn't show a stale approval clock forever.
    stubFetch(
      routeCreate,
      routeStream([
        { type: "RUN_STARTED", turnId: TURN_ID },
        {
          type: "TOOL_CALL_START",
          toolCallStart: { toolCallId: "c1", toolName: "lark_post" },
        },
        {
          type: "TOOL_APPROVAL_REQUEST",
          toolApprovalRequest: { requestId: "req-ledger" },
        },
      ]),
      (url, init) =>
        url.endsWith("/approve") && init?.method === "POST"
          ? jsonResponse({ accepted: true })
          : undefined,
      routeHistory([]),
    );
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();
    await collectTurn(transport, "Post it");
    const history = await transport.getHistory(CONVERSATION_ID);
    const card = history.messages
      .flatMap((message) => message.blocks)
      .find((block) => block.type === "approval_card");

    const events: TurnEvent[] = [];
    await transport.decideApproval(
      CONVERSATION_ID,
      card?.block_id ?? "",
      true,
      (event) => events.push(event),
    );

    const ledgerFlip = events.find(
      (event): event is Extract<TurnEvent, { event: "block.updated" }> =>
        event.event === "block.updated" &&
        (event.patch as { state?: string }).state === "completed",
    );
    expect(ledgerFlip).toBeDefined();
    const steps = (
      ledgerFlip?.patch as {
        steps?: Array<{ status: string }>;
      }
    ).steps;
    expect(steps?.[0]?.status).toBe("done");
  });

  it("settles only the decided approval's step; other gates stay parked", async () => {
    // Second-pass codex P2: ledger settlement is correlated by
    // approval_request_id — deciding one card must not settle a step gated
    // on a different pending approval, and the ledger stays parked.
    stubFetch(
      routeCreate,
      routeStream([
        { type: "RUN_STARTED", turnId: TURN_ID },
        {
          type: "TOOL_CALL_START",
          toolCallStart: { toolCallId: "c1", toolName: "tool_one" },
        },
        {
          type: "TOOL_APPROVAL_REQUEST",
          toolApprovalRequest: { requestId: "req-A", toolCallId: "c1" },
        },
        {
          type: "TOOL_CALL_START",
          toolCallStart: { toolCallId: "c2", toolName: "tool_two" },
        },
        {
          type: "TOOL_APPROVAL_REQUEST",
          toolApprovalRequest: { requestId: "req-B", toolCallId: "c2" },
        },
      ]),
      (url, init) =>
        url.endsWith("/approve") && init?.method === "POST"
          ? jsonResponse({ accepted: true })
          : undefined,
      routeHistory([]),
    );
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();
    await collectTurn(transport, "Two gated actions");
    const history = await transport.getHistory(CONVERSATION_ID);
    const cardB = history.messages
      .flatMap((message) => message.blocks)
      .find(
        (block) =>
          block.type === "approval_card" &&
          block.approval_request_id === "req-B",
      );

    const events: TurnEvent[] = [];
    await transport.decideApproval(
      CONVERSATION_ID,
      cardB?.block_id ?? "",
      true,
      (event) => events.push(event),
    );

    const ledgerPatch = events.find(
      (event): event is Extract<TurnEvent, { event: "block.updated" }> =>
        event.event === "block.updated" &&
        Array.isArray((event.patch as { steps?: unknown }).steps),
    );
    const patch = ledgerPatch?.patch as {
      state?: string;
      steps?: Array<{ status: string; approval_request_id: string | null }>;
    };
    expect(patch.state).toBe("awaiting_approval");
    const stepA = patch.steps?.find(
      (step) => step.approval_request_id === "req-A",
    );
    const stepB = patch.steps?.find(
      (step) => step.approval_request_id === "req-B",
    );
    expect(stepA?.status).toBe("waiting");
    expect(stepB?.status).toBe("done");
  });

  it("terminalizes waiting steps when the run dies at an approval gate", async () => {
    // Second-pass codex P2: a terminal run must not carry a non-terminal
    // step — RUN_ERROR after an approval request skips the waiting step.
    stubFetch(
      routeCreate,
      routeStream([
        { type: "RUN_STARTED", turnId: TURN_ID },
        {
          type: "TOOL_CALL_START",
          toolCallStart: { toolCallId: "c1", toolName: "lark_post" },
        },
        {
          type: "TOOL_APPROVAL_REQUEST",
          toolApprovalRequest: { requestId: "req-dead", toolCallId: "c1" },
        },
        {
          type: "RUN_ERROR",
          runError: { code: "upstream_died", message: "engine crashed" },
        },
      ]),
    );
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    const events = await collectTurn(transport, "Post it");

    const runFinal = blockCompletions(events).find(
      (block) => block.type === "run",
    );
    expect(runFinal?.type === "run" && runFinal.state).toBe("failed");
    expect(runFinal?.type === "run" && runFinal.steps[0]?.status).toBe(
      "skipped",
    );
  });

  it("rejects reads of a deleted conversation instead of resurrecting it", async () => {
    // Second-pass codex P2: history hydration must honor tombstones — a
    // projection racing the delete must not write the row back.
    stubFetch(
      routeCreate,
      routeHistory([
        { id: "h-user", role: "user", content: "hi", timestamp: 1 },
      ]),
      (url, init) =>
        url === `${ASSISTANT_BASE}/conversations/${CONVERSATION_ID}` &&
        init?.method === "DELETE"
          ? jsonResponse({})
          : undefined,
      (url, init) =>
        url === `${ASSISTANT_BASE}/conversations` &&
        (init?.method ?? "GET") === "GET"
          ? jsonResponse({ conversations: [] })
          : undefined,
    );
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    await transport.deleteConversation(CONVERSATION_ID);

    await expect(transport.getHistory(CONVERSATION_ID)).rejects.toThrow(
      "Conversation was not found.",
    );
    expect(await transport.listConversations()).toHaveLength(0);
  });

  it("refuses to serve a pre-delete snapshot when a delete lands mid-read", async () => {
    // Third-pass codex P2: getHistory captures `existing` before awaiting
    // the server; a delete completing during that await must not be
    // answered with the captured pre-delete snapshot via the catch
    // fallback.
    let releaseHistory: () => void = () => {};
    stubFetch(
      routeCreate,
      routeStream(OBSERVED_FRAMES),
      (url, init) =>
        url === `${ASSISTANT_BASE}/conversations/${CONVERSATION_ID}` &&
        init?.method === "DELETE"
          ? jsonResponse({})
          : undefined,
      (url, init) => {
        if (
          url === `${ASSISTANT_BASE}/conversations/${CONVERSATION_ID}` &&
          (init?.method ?? "GET") === "GET"
        ) {
          // Hang the history read until released — the route table cannot
          // express this, so throw a promise-shaped response in via a
          // stub-of-the-stub below.
          return undefined;
        }
        return undefined;
      },
    );
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();
    await collectTurn(transport, "Say hello in five words.");

    const baseFetch = fetch;
    vi.stubGlobal(
      "fetch",
      vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
        if (
          String(input) ===
            `${ASSISTANT_BASE}/conversations/${CONVERSATION_ID}` &&
          (init?.method ?? "GET") === "GET"
        ) {
          return new Promise<Response>((resolve) => {
            releaseHistory = () => {
              resolve(jsonResponse([]));
            };
          });
        }
        return baseFetch(input, init);
      }),
    );

    const pendingRead = transport.getHistory(CONVERSATION_ID);
    await new Promise((resolve) => setTimeout(resolve, 0));
    await transport.deleteConversation(CONVERSATION_ID);
    releaseHistory();

    await expect(pendingRead).rejects.toThrow("Conversation was not found.");
  });

  it("answers not-found, not the read failure, when a delete lands mid-read and the read then fails", async () => {
    // Fourth-pass codex P2: narrowing the getHistory catch added two early
    // throws that could bypass the post-await tombstone check. Delete must
    // still win — with an index-only mirror (EMPTY_TURN_STATE), a read that
    // rejects after the delete must report "not found", not the transport
    // failure.
    let rejectHistory: (error: Error) => void = () => {};
    vi.stubGlobal(
      "fetch",
      vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        const method = init?.method ?? "GET";
        if (url === `${ASSISTANT_BASE}/conversations` && method === "GET") {
          return Promise.resolve(
            jsonResponse({
              conversations: [{ id: CONVERSATION_ID, title: "Server title" }],
            }),
          );
        }
        if (
          url === `${ASSISTANT_BASE}/conversations/${CONVERSATION_ID}` &&
          method === "DELETE"
        ) {
          return Promise.resolve(jsonResponse({}));
        }
        if (
          url === `${ASSISTANT_BASE}/conversations/${CONVERSATION_ID}` &&
          method === "GET"
        ) {
          return new Promise<Response>((_resolve, reject) => {
            rejectHistory = reject;
          });
        }
        return Promise.resolve(jsonResponse({}, 404));
      }),
    );
    const transport = new AevatarAssistantTransport();
    // Index-only mirror: no turn ever ran, so `turnState` is EMPTY_TURN_STATE.
    await transport.listConversations();

    const pendingRead = transport.getHistory(CONVERSATION_ID);
    await new Promise((resolve) => setTimeout(resolve, 0));
    await transport.deleteConversation(CONVERSATION_ID);
    rejectHistory(new Error("network down"));

    await expect(pendingRead).rejects.toThrow("Conversation was not found.");
  });

  it("stops an approve request hung before response headers", async () => {
    // Second-pass codex P2: Stop works during the pre-header window via the
    // transport-level cancel (the caller holds no handle yet).
    stubFetch(
      routeCreate,
      routeStream([
        { type: "RUN_STARTED", turnId: TURN_ID },
        {
          type: "TOOL_APPROVAL_REQUEST",
          toolApprovalRequest: { requestId: "req-hung" },
        },
      ]),
      routeHistory([]),
    );
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();
    await collectTurn(transport, "Do it");
    const history = await transport.getHistory(CONVERSATION_ID);
    const card = history.messages
      .flatMap((message) => message.blocks)
      .find((block) => block.type === "approval_card");

    const baseFetch = fetch;
    vi.stubGlobal(
      "fetch",
      vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
        if (String(input).endsWith("/approve")) {
          // Hang until aborted.
          return new Promise<Response>((_resolve, reject) => {
            init?.signal?.addEventListener("abort", () => {
              reject(new DOMException("aborted", "AbortError"));
            });
          });
        }
        return baseFetch(input, init);
      }),
    );

    const events: TurnEvent[] = [];
    const pending = transport.decideApproval(
      CONVERSATION_ID,
      card?.block_id ?? "",
      true,
      (event) => events.push(event),
    );
    await new Promise((resolve) => setTimeout(resolve, 0));
    transport.cancelActiveTurn(CONVERSATION_ID);

    await expect(pending).rejects.toThrow("The approval request was stopped.");
    const terminal = events[events.length - 1];
    expect(terminal?.event === "turn.completed" && terminal.status).toBe(
      "cancelled",
    );
  });

  it("fails an approve rejection with a single messaging surface", async () => {
    // Second-pass codex P2: pre-stream approve failures reject the mutation
    // (its toast) and settle the turn with a NULL error so the generic
    // reply-failed toast cannot double-fire.
    stubFetch(
      routeCreate,
      routeStream([
        { type: "RUN_STARTED", turnId: TURN_ID },
        {
          type: "TOOL_APPROVAL_REQUEST",
          toolApprovalRequest: { requestId: "req-502" },
        },
      ]),
      (url, init) =>
        url.endsWith("/approve") && init?.method === "POST"
          ? jsonResponse({ code: "UPSTREAM_DOWN", message: "bad gateway" }, 502)
          : undefined,
      routeHistory([]),
    );
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();
    await collectTurn(transport, "Do it");
    const history = await transport.getHistory(CONVERSATION_ID);
    const card = history.messages
      .flatMap((message) => message.blocks)
      .find((block) => block.type === "approval_card");

    const events: TurnEvent[] = [];
    await expect(
      transport.decideApproval(
        CONVERSATION_ID,
        card?.block_id ?? "",
        true,
        (event) => events.push(event),
      ),
    ).rejects.toThrow("bad gateway");

    const terminal = events[events.length - 1];
    expect(
      terminal?.event === "turn.completed" && {
        status: terminal.status,
        error: terminal.error,
      },
    ).toEqual({ status: "failed", error: null });
    // The card was never flipped — the decision stays retryable.
    const after = await transport.getHistory(CONVERSATION_ID);
    const cardAfter = after.messages
      .flatMap((message) => message.blocks)
      .find((block) => block.type === "approval_card");
    expect(cardAfter?.type === "approval_card" && cardAfter.decision).toBe(
      null,
    );
  });

  it("mines a raw.observed completion for steps and fallback text, never reasoning", async () => {
    stubFetch(
      routeCreate,
      routeStream([
        { type: "RUN_STARTED", turnId: TURN_ID },
        {
          type: "CUSTOM",
          custom: {
            name: "aevatar.raw.observed",
            payload: {
              "@type":
                "type.googleapis.com/aevatar.workflow.runs.WorkflowObservedEnvelopeCustomPayload",
              eventId: "evt-1",
              payloadTypeUrl:
                "type.googleapis.com/aevatar.ai.RoleChatSessionCompletedEvent",
              payload: {
                "@type":
                  "type.googleapis.com/aevatar.ai.RoleChatSessionCompletedEvent",
                sessionId: "session-1",
                content: "All done.",
                reasoningContent: "private trace",
                toolCalls: [
                  {
                    callId: "call-1",
                    toolName: "ornn_search_skills",
                    argumentsJson: '{"query":"nyxid"}',
                  },
                ],
                toolReceipts: [
                  {
                    callId: "call-1",
                    toolName: "ornn_search_skills",
                    status: "AGENT_TOOL_RECEIPT_STATUS_SUCCESS",
                    resultJson: '{"found":true}',
                  },
                ],
                usage: { promptTokens: 100, completionTokens: 20 },
                model: "gpt-test",
              },
            },
          },
        },
        { type: "RUN_FINISHED" },
      ]),
      routeHistory([]),
    );
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    const events = await collectTurn(transport, "Search and finish");

    const runFinal = blockCompletions(events).find(
      (block) => block.type === "run",
    );
    expect(runFinal?.type === "run" && runFinal.steps[0]).toMatchObject({
      status: "done",
      label: "ornn_search_skills",
    });
    const text = blockCompletions(events).find(
      (block) => block.type === "text",
    );
    expect(text?.type === "text" && text.text).toBe("All done.");
    // PRD §3.8: engine reasoning never reaches a rendered event.
    expect(JSON.stringify(events)).not.toContain("private trace");
    const history = await transport.getHistory(CONVERSATION_ID);
    expect(history.conversation.llm_model).toBe("gpt-test");
  });

  it("treats RUN_STOPPED as a terminal stop, not a truncated stream", async () => {
    stubFetch(
      routeCreate,
      routeStream([
        { type: "RUN_STARTED", turnId: TURN_ID },
        {
          type: "TEXT_MESSAGE_START",
          textMessageStart: { messageId: "m-1", role: "assistant" },
        },
        {
          type: "TEXT_MESSAGE_CONTENT",
          textMessageContent: { delta: "Partial" },
        },
        { type: "RUN_STOPPED", runStopped: { reason: "operator" } },
      ]),
    );
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    const events = await collectTurn(transport, "Do something long");

    const terminal = events[events.length - 1];
    expect(
      terminal?.event === "turn.completed" && {
        status: terminal.status,
        error: terminal.error,
      },
    ).toEqual({ status: "cancelled", error: null });
  });

  it("accepts body-keyed frames with no type tag", async () => {
    // The reference client accepts either shape for every frame family; a
    // body-keyed terminal frame falling through to UNKNOWN would leave the
    // turn looking truncated when it actually completed.
    stubFetch(
      routeCreate,
      routeStream([
        { runStarted: { turnId: TURN_ID, actorId: CONVERSATION_ID } },
        { textMessageStart: { messageId: "m-1", role: "assistant" } },
        { textMessageContent: { delta: "Body-keyed hello" } },
        { stepStarted: { stepName: "plan" } },
        { stepFinished: { stepName: "plan", success: true } },
        { textMessageEnd: { messageId: "m-1" } },
        { runFinished: {} },
      ]),
    );
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    const events = await collectTurn(transport, "Hello");

    const terminal = events[events.length - 1];
    expect(
      terminal?.event === "turn.completed" && {
        status: terminal.status,
        error: terminal.error,
      },
    ).toEqual({ status: "completed", error: null });
    const text = blockCompletions(events).find(
      (block) => block.type === "text",
    );
    expect(text?.type === "text" && text.text).toBe("Body-keyed hello");
    const runFinal = blockCompletions(events).find(
      (block) => block.type === "run",
    );
    expect(runFinal?.type === "run" && runFinal.steps[0]).toMatchObject({
      status: "done",
      label: "plan",
    });
  });

  it("maps top-level STEP_STARTED/STEP_FINISHED onto the run ledger", async () => {
    stubFetch(
      routeCreate,
      routeStream([
        { type: "RUN_STARTED", turnId: TURN_ID },
        { type: "STEP_STARTED", stepStarted: { stepName: "collect" } },
        {
          type: "STEP_FINISHED",
          stepFinished: { stepName: "collect", success: false },
        },
        { type: "RUN_FINISHED" },
      ]),
    );
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    const events = await collectTurn(transport, "Run the workflow");

    const runFinal = blockCompletions(events).find(
      (block) => block.type === "run",
    );
    expect(runFinal?.type === "run" && runFinal.steps[0]).toMatchObject({
      status: "failed",
      label: "collect",
    });
    expect(runFinal?.type === "run" && runFinal.state).toBe("failed");
  });

  it("maps workflow step customs onto the run ledger", async () => {
    stubFetch(
      routeCreate,
      routeStream([
        { type: "RUN_STARTED", turnId: TURN_ID },
        {
          type: "CUSTOM",
          custom: {
            name: "aevatar.step.request",
            payload: { runId: "run-1", stepId: "plan" },
          },
        },
        {
          type: "CUSTOM",
          custom: {
            name: "aevatar.step.completed",
            payload: { runId: "run-1", stepId: "plan", success: true },
          },
        },
        { type: "RUN_FINISHED" },
      ]),
    );
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    const events = await collectTurn(transport, "Plan it");

    const runFinal = blockCompletions(events).find(
      (block) => block.type === "run",
    );
    expect(runFinal?.type === "run" && runFinal.state).toBe("completed");
    expect(runFinal?.type === "run" && runFinal.steps[0]).toMatchObject({
      status: "done",
      label: "plan",
    });
  });

  it("embeds MEDIA_CONTENT as a data-URL artifact block", async () => {
    stubFetch(
      routeCreate,
      routeStream([
        { type: "RUN_STARTED", turnId: TURN_ID },
        {
          type: "MEDIA_CONTENT",
          mediaContent: {
            mediaType: "image/png",
            dataBase64: "aGVsbG8=",
            name: "chart.png",
          },
        },
        { type: "RUN_FINISHED" },
      ]),
    );
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    const events = await collectTurn(transport, "Chart it");

    const artifact = blockStarts(events).find(
      (block) => block.type === "artifact",
    );
    expect(artifact?.type === "artifact" && artifact).toMatchObject({
      name: "chart.png",
      mime: "image/png",
      size_bytes: 6,
      download_url: "data:image/png;base64,aGVsbG8=",
    });
  });

  it("fails a run stalled on keepalives once the 120s watchdog fires", async () => {
    vi.useFakeTimers();
    try {
      const encoder = new TextEncoder();
      let push: (frame: unknown) => void = () => {};
      const openStream = new ReadableStream<Uint8Array>({
        start(controller) {
          push = (frame) => {
            controller.enqueue(
              encoder.encode(`data: ${JSON.stringify(frame)}\n\n`),
            );
          };
          push({ type: "RUN_STARTED", turnId: TURN_ID });
        },
      });
      stubFetch(routeCreate, (url, init) =>
        url.endsWith("/stream") && init?.method === "POST"
          ? new Response(openStream, {
              status: 200,
              headers: { "Content-Type": "text/event-stream" },
            })
          : undefined,
      );
      const transport = new AevatarAssistantTransport();
      await transport.createConversation();

      const events: TurnEvent[] = [];
      const done = new Promise<void>((resolve) => {
        transport.sendMessage(CONVERSATION_ID, "Hello", (event) => {
          events.push(event);
          if (event.event === "turn.completed") resolve();
        });
      });
      // Keepalives keep the socket open but are not progress.
      await vi.advanceTimersByTimeAsync(60_000);
      push?.({
        type: "CUSTOM",
        custom: { name: "aevatar.nyxid_chat.keepalive", payload: {} },
      });
      await vi.advanceTimersByTimeAsync(60_001);
      await done;

      const terminal = events[events.length - 1];
      expect(terminal?.event === "turn.completed" && terminal.status).toBe(
        "failed",
      );
      expect(terminal?.event === "turn.completed" && terminal.error?.code).toBe(
        "upstream_progress_timeout",
      );
    } finally {
      vi.useRealTimers();
    }
  });

  it("re-arms the watchdog on real progress frames", async () => {
    vi.useFakeTimers();
    try {
      const encoder = new TextEncoder();
      let push: (frame: unknown) => void = () => {};
      const openStream = new ReadableStream<Uint8Array>({
        start(controller) {
          push = (frame) => {
            controller.enqueue(
              encoder.encode(`data: ${JSON.stringify(frame)}\n\n`),
            );
          };
          push({ type: "RUN_STARTED", turnId: TURN_ID });
        },
      });
      stubFetch(routeCreate, (url, init) =>
        url.endsWith("/stream") && init?.method === "POST"
          ? new Response(openStream, {
              status: 200,
              headers: { "Content-Type": "text/event-stream" },
            })
          : undefined,
      );
      const transport = new AevatarAssistantTransport();
      await transport.createConversation();

      const events: TurnEvent[] = [];
      const done = new Promise<void>((resolve) => {
        transport.sendMessage(CONVERSATION_ID, "Hello", (event) => {
          events.push(event);
          if (event.event === "turn.completed") resolve();
        });
      });
      await vi.advanceTimersByTimeAsync(100_000);
      push?.({
        type: "TEXT_MESSAGE_START",
        textMessageStart: { messageId: "m-1", role: "assistant" },
      });
      // 100s after the reset: under the 120s deadline, so still running.
      await vi.advanceTimersByTimeAsync(100_000);
      expect(events.some((event) => event.event === "turn.completed")).toBe(
        false,
      );
      await vi.advanceTimersByTimeAsync(20_001);
      await done;

      const terminal = events[events.length - 1];
      expect(terminal?.event === "turn.completed" && terminal.error?.code).toBe(
        "upstream_progress_timeout",
      );
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("redaction and tool summaries", () => {
  it("redacts secret assignments and token shapes from display text", () => {
    expect(
      redactDisplayText('{"api_key":"sk-live-12345","status":"ok"}'),
    ).not.toContain("sk-live-12345");
    expect(
      redactDisplayText("Authorization: Bearer abc.def.ghi"),
    ).not.toContain("abc.def.ghi");
    expect(redactDisplayText("key nyxid_ag_supersecret1234")).toContain(
      "[redacted]",
    );
    // Codex P1 coverage gaps: Basic auth values, pre/suffixed key names,
    // and quoted secrets containing spaces.
    expect(
      redactDisplayText("Authorization: Basic dXNlcjpwYXNzd29yZA=="),
    ).not.toContain("dXNlcjpwYXNzd29yZA");
    expect(
      redactDisplayText('{"secretAccessKey":"wJalrXUtnFEMI/K7MDENG"}'),
    ).not.toContain("wJalrXUtnFEMI");
    expect(
      redactDisplayText('{"accessKeyId":"AKIAIOSFODNN7EXAMPLE"}'),
    ).not.toContain("AKIAIOSFODNN7");
    expect(
      redactDisplayText('{"password": "correct horse battery staple"}'),
    ).not.toContain("horse battery");
    expect(redactDisplayText('{"x-api-key":"abc123secret"}')).not.toContain(
      "abc123secret",
    );
    // Second-pass codex P1: bare token shapes in natural-language strings
    // (no key=value assignment present) and single-quoted assignments.
    expect(
      redactDisplayText("AWS rejected AKIAIOSFODNN7EXAMPLE for this call"),
    ).not.toContain("AKIAIOSFODNN7EXAMPLE");
    expect(
      redactDisplayText("OpenAI rejected sk-proj-abc123def456ghi"),
    ).not.toContain("sk-proj-abc123def456ghi");
    expect(
      redactDisplayText("push failed for ghp_abcdefghij1234567890KLMN"),
    ).not.toContain("ghp_abcdefghij1234567890KLMN");
    expect(
      redactDisplayText("{'api_key': 'sk-live-secret with spaces'}"),
    ).not.toContain("sk-live-secret");
  });

  it("redacts credentials from RUN_ERROR messages before they render", async () => {
    stubFetch(
      routeCreate,
      routeStream([
        { type: "RUN_STARTED", turnId: TURN_ID },
        {
          type: "RUN_ERROR",
          runError: {
            code: "upstream_error",
            message:
              "Downstream 401 with Authorization: Bearer sk.live.abc123 header",
          },
        },
      ]),
    );
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    const events = await collectTurn(transport, "Hello");

    const terminal = events[events.length - 1];
    const message =
      terminal?.event === "turn.completed" ? terminal.error?.message : "";
    expect(message).not.toContain("sk.live.abc123");
    expect(message).toContain("[redacted]");
  });

  it("summarizes tool results as compact redacted single lines", () => {
    expect(summarizeToolResult(undefined)).toBe("Completed");
    expect(summarizeToolResult({ found: true })).toBe('{"found":true}');
    const long = summarizeToolResult({ text: "x".repeat(500) });
    expect(long.length).toBeLessThanOrEqual(160);
    expect(long.endsWith("…")).toBe(true);
  });
});

describe("transport selection", () => {
  it("selects aevatar for production sessions", () => {
    expect(
      selectAssistantTransportKind({
        mode: "production",
        dev: false,
        search: "",
      }),
    ).toBe("aevatar");
  });

  it("ignores ?mock outside dev builds", () => {
    expect(
      selectAssistantTransportKind({
        mode: "production",
        dev: false,
        search: "?mock",
      }),
    ).toBe("aevatar");
  });

  it("selects the scripted mock for dev ?mock sessions and vitest", () => {
    expect(
      selectAssistantTransportKind({
        mode: "development",
        dev: true,
        search: "?mock",
      }),
    ).toBe("mock");
    expect(
      selectAssistantTransportKind({ mode: "test", dev: true, search: "" }),
    ).toBe("mock");
  });

  it("selects aevatar for plain dev sessions", () => {
    expect(
      selectAssistantTransportKind({
        mode: "development",
        dev: true,
        search: "",
      }),
    ).toBe("aevatar");
  });
});

describe("connect markers in assistant messages", () => {
  const MARKER = [
    "```nyxid:connect",
    JSON.stringify({ catalog_slug: "api-github", reason: "read merged PRs" }),
    "```",
  ].join("\n");

  function connectCards(blocks: readonly ContentBlock[]): ContentBlock[] {
    return blocks.filter((block) => block.type === "connect_card");
  }

  it("renders cards from stored history — the reload path", async () => {
    // The whole point of the in-text encoding: the FE may call only chat and
    // history, so a card that lived in a live SSE frame is gone after a
    // reload. Re-splitting the stored string brings it back.
    stubFetch(
      routeHistory([
        {
          id: "m1",
          role: "user",
          content: "Summarise my merged PRs",
          timestamp: 1784192889074,
        },
        {
          id: "m2",
          role: "assistant",
          content: `I need GitHub first.\n${MARKER}\nThen I'll summarise.`,
          timestamp: 1784192899074,
        },
      ]),
    );
    const transport = new AevatarAssistantTransport();

    const history = await transport.getHistory(CONVERSATION_ID);

    const assistant = history.messages.find((m) => m.id === "m2");
    expect(assistant?.blocks.map((b) => b.type)).toEqual([
      "text",
      "connect_card",
      "text",
    ]);
    const card = connectCards(assistant?.blocks ?? [])[0];
    expect(card).toMatchObject({
      type: "connect_card",
      catalog_slug: "api-github",
      state: "needs_connection",
    });
    // The raw encoding must never survive into rendered prose.
    for (const block of assistant?.blocks ?? []) {
      if (block.type === "text") {
        expect(block.text).not.toContain("nyxid:connect");
      }
    }
  });

  it("never mints a card from a user message", async () => {
    // Markers are Aevatar-authored. A user pasting the encoding into chat
    // must not be able to conjure a connect card.
    stubFetch(
      routeHistory([
        { id: "m1", role: "user", content: MARKER, timestamp: 1784192889074 },
      ]),
    );
    const transport = new AevatarAssistantTransport();

    const history = await transport.getHistory(CONVERSATION_ID);

    expect(connectCards(history.messages[0]?.blocks ?? [])).toHaveLength(0);
  });

  it("leaves marker-free history byte-identical", async () => {
    stubFetch(
      routeHistory([
        {
          id: "m1",
          role: "assistant",
          content: "Nothing to connect here.",
          timestamp: 1784192889074,
        },
      ]),
    );
    const transport = new AevatarAssistantTransport();

    const history = await transport.getHistory(CONVERSATION_ID);

    expect(history.messages[0]?.blocks).toEqual([
      { type: "text", block_id: "m1-text", text: "Nothing to connect here." },
    ]);
  });

  it("emits a card from the live stream and keeps the encoding out of prose", async () => {
    stubFetch(
      routeCreate,
      routeStream([
        { type: "RUN_STARTED", turnId: TURN_ID, actorId: CONVERSATION_ID },
        { type: "TEXT_MESSAGE_START", textMessageStart: { messageId: "m9" } },
        {
          type: "TEXT_MESSAGE_CONTENT",
          textMessageContent: { delta: "I need GitHub first.\n" },
        },
        { type: "TEXT_MESSAGE_CONTENT", textMessageContent: { delta: MARKER } },
        { type: "TEXT_MESSAGE_END" },
        { type: "RUN_FINISHED" },
      ]),
      routeHistory([]),
    );
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    const events = await collectTurn(transport, "Summarise my merged PRs");

    const started = events
      .filter(
        (e): e is Extract<TurnEvent, { event: "block.started" }> =>
          e.event === "block.started",
      )
      .map((e) => e.block);
    expect(connectCards(started)).toHaveLength(1);
    expect(connectCards(started)[0]).toMatchObject({
      catalog_slug: "api-github",
    });

    // No streamed delta may carry the raw encoding.
    for (const event of events) {
      if (event.event === "block.delta") {
        expect(event.text).not.toContain("nyxid:connect");
      }
    }
  });

  it("withholds a half-written marker while it streams", async () => {
    stubFetch(
      routeCreate,
      routeStream([
        { type: "RUN_STARTED", turnId: TURN_ID, actorId: CONVERSATION_ID },
        { type: "TEXT_MESSAGE_START", textMessageStart: { messageId: "m9" } },
        {
          type: "TEXT_MESSAGE_CONTENT",
          textMessageContent: { delta: "Hold on.\n``" },
        },
        {
          type: "TEXT_MESSAGE_CONTENT",
          textMessageContent: { delta: "`nyxid:conn" },
        },
        {
          type: "TEXT_MESSAGE_CONTENT",
          textMessageContent: { delta: `ect\n{"catalog_slug":"api-github"}\n` },
        },
        { type: "TEXT_MESSAGE_CONTENT", textMessageContent: { delta: "```" } },
        { type: "TEXT_MESSAGE_END" },
        { type: "RUN_FINISHED" },
      ]),
      routeHistory([]),
    );
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    const events = await collectTurn(transport, "go");

    // Deltas are append-only prose; the partial fence never leaks, so the
    // transcript never shows text it must later retract.
    const streamed = events
      .filter(
        (e): e is Extract<TurnEvent, { event: "block.delta" }> =>
          e.event === "block.delta",
      )
      .map((e) => e.text)
      .join("");
    expect(streamed.trim()).toBe("Hold on.");
    expect(streamed).not.toContain("nyxid");
    expect(streamed).not.toContain("`");

    const started = events
      .filter(
        (e): e is Extract<TurnEvent, { event: "block.started" }> =>
          e.event === "block.started",
      )
      .map((e) => e.block);
    expect(connectCards(started)).toHaveLength(1);
  });
});

describe("live/history convergence for connect markers", () => {
  // The design rests on one property: what a live turn renders and what a
  // reload replays must be the same blocks. Codex's review found they were
  // not — the live path derived its own ordering and ids. These cases pin it.
  const CASES: ReadonlyArray<readonly [string, string]> = [
    [
      "marker first",
      '```nyxid:connect\n{"catalog_slug":"api-github"}\n```\nAfter',
    ],
    [
      "marker last",
      'Before\n```nyxid:connect\n{"catalog_slug":"api-github"}\n```',
    ],
    ["marker only", '```nyxid:connect\n{"catalog_slug":"api-github"}\n```'],
    [
      "text card text",
      'Before\n```nyxid:connect\n{"catalog_slug":"api-github"}\n```\nAfter',
    ],
    [
      "adjacent markers",
      '```nyxid:connect\n{"catalog_slug":"api-github"}\n```\n```nyxid:connect\n{"catalog_slug":"api-lark-bot"}\n```',
    ],
    ["no marker", "Plain prose only."],
  ];

  /** Split into `size`-char deltas — real streams do not arrive whole. */
  function chunks(content: string, size: number): string[] {
    if (size <= 0) return [content];
    const out: string[] = [];
    for (let at = 0; at < content.length; at += size) {
      out.push(content.slice(at, at + size));
    }
    return out.length > 0 ? out : [""];
  }

  // Chunk size 0 = one whole delta. 1 exercises the worst case: every marker
  // character arrives separately, so the partial-marker guard and the suffix
  // diff are hit at every boundary.
  const CHUNKINGS = [0, 1, 3, 7];

  it.each(
    CASES.flatMap(([label, content]) =>
      CHUNKINGS.map(
        (size) =>
          [`${label} (chunk ${String(size)})`, content, size] as const,
      ),
    ),
  )("%s", async (_label, content, chunkSize) => {
    stubFetch(
      routeCreate,
      routeStream([
        { type: "RUN_STARTED", turnId: TURN_ID, actorId: CONVERSATION_ID },
        { type: "TEXT_MESSAGE_START", textMessageStart: { messageId: "mX" } },
        ...chunks(content, chunkSize).map((delta) => ({
          type: "TEXT_MESSAGE_CONTENT",
          textMessageContent: { delta },
        })),
        { type: "TEXT_MESSAGE_END" },
        { type: "RUN_FINISHED" },
      ]),
      routeHistory([]),
    );
    const liveTransport = new AevatarAssistantTransport();
    await liveTransport.createConversation();
    const events = await collectTurn(liveTransport, "go");

    // Final state of every block this message produced, in emission order.
    const order: string[] = [];
    const finals = new Map<string, ContentBlock>();
    for (const event of events) {
      if (event.event === "block.completed") {
        if (!finals.has(event.block_id)) order.push(event.block_id);
        finals.set(event.block_id, event.block);
      }
    }
    const liveBlocks = order
      .filter((id) => id.startsWith("mX"))
      .map((id) => finals.get(id));

    // History: the same text, replayed through the reload path.
    stubFetch(
      routeHistory([
        {
          id: "mX",
          role: "assistant",
          content,
          timestamp: 1784192899074,
        },
      ]),
    );
    const historyTransport = new AevatarAssistantTransport();
    const history = await historyTransport.getHistory(CONVERSATION_ID);
    const historyBlocks = history.messages.find((m) => m.id === "mX")?.blocks;

    expect(liveBlocks).toEqual(historyBlocks);

    // Whatever the chunking, raw marker syntax must never reach the stream.
    for (const event of events) {
      if (event.event === "block.delta") {
        expect(event.text).not.toContain("nyxid:connect");
      }
    }
  });
});

describe("cancel runs the same projection as a normal close", () => {
  const MARKER = [
    "```nyxid:connect",
    JSON.stringify({ catalog_slug: "api-github", reason: "read PRs" }),
    "```",
  ].join("\n");

  /** A stream that emits some deltas then stays open until cancelled. */
  function hangingStream(deltas: readonly string[]): FetchRoute {
    return (url, init) => {
      if (!url.endsWith("/stream") || init?.method !== "POST") return undefined;
      const body = new ReadableStream<Uint8Array>({
        start(controller) {
          const encoder = new TextEncoder();
          const frames = [
            { type: "RUN_STARTED", turnId: TURN_ID, actorId: CONVERSATION_ID },
            { type: "TEXT_MESSAGE_START", textMessageStart: { messageId: "mC" } },
            ...deltas.map((delta) => ({
              type: "TEXT_MESSAGE_CONTENT",
              textMessageContent: { delta },
            })),
          ];
          controller.enqueue(
            encoder.encode(
              frames.map((f) => `data: ${JSON.stringify(f)}\n\n`).join(""),
            ),
          );
        },
      });
      return new Response(body, {
        status: 200,
        headers: { "Content-Type": "text/event-stream" },
      });
    };
  }

  async function cancelAfterDeltas(
    deltas: readonly string[],
  ): Promise<TurnEvent[]> {
    stubFetch(routeCreate, hangingStream(deltas), routeHistory([]));
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();
    const events: TurnEvent[] = [];
    let seen = 0;
    await new Promise<void>((resolve) => {
      const handle = transport.sendMessage(CONVERSATION_ID, "go", (event) => {
        events.push(event);
        if (event.event === "turn.completed") resolve();
        if (event.event === "block.delta") {
          seen += 1;
          // Cancel once every delta the transport chose to emit has landed.
          if (seen >= 1) setTimeout(() => handle.cancel(), 0);
        }
      });
      // A message that is only a marker emits no delta at all — cancel anyway.
      setTimeout(() => handle.cancel(), 20);
    });
    return events;
  }

  it("emits the card when a turn is cancelled after a complete marker", async () => {
    // Previously cancel bypassed the splitter entirely: no card was emitted
    // live, but a reload produced one — the two disagreed.
    const events = await cancelAfterDeltas([`Need GitHub.\n${MARKER}`]);

    const blocks = events
      .filter(
        (e): e is Extract<TurnEvent, { event: "block.completed" }> =>
          e.event === "block.completed",
      )
      .map((e) => e.block);
    expect(blocks.some((block) => block.type === "connect_card")).toBe(true);
  });

  it("never leaks raw marker syntax when cancelled mid-message", async () => {
    const events = await cancelAfterDeltas([`Need GitHub.\n${MARKER}`]);

    for (const event of events) {
      if (event.event === "block.delta") {
        expect(event.text).not.toContain("nyxid:connect");
      }
      if (event.event === "block.completed" && event.block.type === "text") {
        expect(event.block.text).not.toContain("nyxid:connect");
      }
    }
  });
});

describe("connect markers survive the PR #2923 wrapped transcript", () => {
  // Merge-point coverage: main taught `loadHistory` to accept
  // `{messages, stateVersion}`; this branch taught `historyEntryToMessage` to
  // project markers into blocks. Neither side tested the two together, and the
  // reload path — the whole reason the marker is carried in the text — runs
  // through both.
  const MARKER = [
    "```nyxid:connect",
    JSON.stringify({ catalog_slug: "api-github", reason: "read PRs" }),
    "```",
  ].join("\n");

  const ENTRIES = [
    {
      id: "w1",
      role: "assistant",
      content: `Need GitHub.\n${MARKER}\nThen I'll continue.`,
      timestamp: 1784192899074,
    },
  ];

  it("renders the card from the wrapped shape", async () => {
    stubFetch((url, init) =>
      url.startsWith(`${ASSISTANT_BASE}/conversations/`) &&
      (init?.method ?? "GET") === "GET"
        ? jsonResponse({ messages: ENTRIES, stateVersion: 42 })
        : undefined,
    );
    const transport = new AevatarAssistantTransport();

    const history = await transport.getHistory(CONVERSATION_ID);

    const blocks = history.messages.find((m) => m.id === "w1")?.blocks ?? [];
    expect(blocks.map((b) => b.type)).toEqual([
      "text",
      "connect_card",
      "text",
    ]);
  });

  it("renders identically from the legacy flat array", async () => {
    stubFetch(routeHistory(ENTRIES));
    const transport = new AevatarAssistantTransport();

    const history = await transport.getHistory(CONVERSATION_ID);

    const blocks = history.messages.find((m) => m.id === "w1")?.blocks ?? [];
    expect(blocks.map((b) => b.type)).toEqual([
      "text",
      "connect_card",
      "text",
    ]);
  });
});

describe("a conversation with no committed turn has no server transcript", () => {
  // The two upstream halves materialize at different times: `nyxid-chat`
  // mints the actor at create, while the `chat-history` row is only written
  // when a turn reaches a terminal. Aevatar's `HandleGetConversation` maps a
  // missing read-model document to 404 — NOT to an empty array — so a
  // freshly created conversation 404s here by design. Treating that as a
  // failure is what made every new chat dead-end on "Failed to load this
  // conversation" (observed in prod, 2026-07-28).
  it("serves an empty transcript when the history row does not exist yet", async () => {
    // Only the create is routed; stubFetch answers everything else 404.
    stubFetch(routeCreate);
    const transport = new AevatarAssistantTransport();
    const conversation = await transport.createConversation();

    const history = await transport.getHistory(conversation.id);

    expect(history.messages).toEqual([]);
    expect(history.conversation.id).toBe(CONVERSATION_ID);
  });

  it("still rejects a 404 for a conversation it has never seen", async () => {
    stubFetch(routeCreate);
    const transport = new AevatarAssistantTransport();

    await expect(
      transport.getHistory("nyxid-chat-never-created"),
    ).rejects.toThrow();
  });

  it("still rejects a transient failure on a conversation with no local transcript", async () => {
    // 5xx is a real read failure, not the not-yet-materialized state — it
    // must not be dressed up as a legitimately empty chat.
    stubFetch(routeCreate, (url, init) =>
      url.startsWith(`${ASSISTANT_BASE}/conversations/`) &&
      (init?.method ?? "GET") === "GET"
        ? jsonResponse(
            { error: "internal", error_code: -1, message: "boom" },
            500,
          )
        : undefined,
    );
    const transport = new AevatarAssistantTransport();
    const conversation = await transport.createConversation();

    await expect(transport.getHistory(conversation.id)).rejects.toThrow();
  });
});
