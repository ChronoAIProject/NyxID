import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AevatarAssistantTransport } from "@/lib/assistant/aevatar-transport";
import { AssistantTurnActiveError } from "@/lib/assistant/errors";
import { selectAssistantTransportKind } from "@/lib/assistant/transport";
import capturedHistory from "@/lib/assistant/__fixtures__/aevatar-chat-history.json";
import capturedStream from "@/lib/assistant/__fixtures__/aevatar-nyxid-chat-stream.sse?raw";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
import type { TurnEvent } from "@/types/assistant";

const USER_ID = "add69059-bece-4f0e-9559-99cfd10b47eb";
const CONVERSATION_ID = "nyxid-chat-f8369965a444433f92ec50e67ad8ee52";
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

// The exact frame sequence observed live against aevatar's
// `nyxid-chat/conversations/{id}:stream` on 2026-07-16.
const OBSERVED_FRAMES = [
  { type: "RUN_STARTED", actorId: CONVERSATION_ID },
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

function routeHistory(entries: unknown[]): FetchRoute {
  return (url, init) =>
    url.startsWith(`${ASSISTANT_BASE}/conversations/`) &&
    (init?.method ?? "GET") === "GET"
      ? jsonResponse(entries)
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
        { type: "RUN_STARTED" },
        {
          type: "RUN_ERROR",
          error: { code: "upstream_timeout", message: "Model timed out" },
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
    expect(
      terminal?.event === "turn.completed" && terminal.error?.code,
    ).toBe("turn_active");
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
    expect(
      terminal?.event === "turn.completed" && terminal.error?.code,
    ).toBe("stream_closed");
    const history = await transport.getHistory(CONVERSATION_ID);
    expect(history.messages[1]?.blocks).toEqual([
      { type: "text", block_id: "m-1-text", text: "Hello, " },
    ]);
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
      const handle = transport.sendMessage(CONVERSATION_ID, "Hello", (event) => {
        events.push(event);
        if (event.event === "turn.completed") resolve();
        if (event.event === "block.delta") handle.cancel();
      });
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
    const bytes = new TextEncoder().encode(capturedStream);
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
    const completed = events.find(
      (event) => event.event === "block.completed",
    );
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
      sessionId: string;
    };
    expect(body.prompt).toBe("Hello there");
    // Run-session correlation id (optional upstream; the reference client
    // sends one per conversation).
    expect(body.sessionId).toMatch(
      /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/,
    );
  });

  it("keeps one sessionId per conversation across turns", async () => {
    const fetchMock = stubFetch(routeCreate, routeStream(OBSERVED_FRAMES));
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    await collectTurn(transport, "First turn");
    await collectTurn(transport, "Second turn");

    const sessionIds = fetchMock.mock.calls
      .filter(([input]) => String(input).endsWith("/stream"))
      .map(
        ([, init]) =>
          (JSON.parse(String(init?.body)) as { sessionId: string }).sessionId,
      );
    expect(sessionIds).toHaveLength(2);
    expect(sessionIds[0]).toBe(sessionIds[1]);
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
