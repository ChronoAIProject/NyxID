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

function actionRequestFrame(
  overrides: Record<string, unknown> = {},
): Record<string, unknown> {
  return {
    type: "CUSTOM",
    custom: {
      name: "nyxid.action.request",
      payload: {
        schemaVersion: 4,
        actorId: CONVERSATION_ID,
        originTurnId: TURN_ID,
        taskId: "task-action-1",
        stepId: "step-action-1",
        actionRequestId: "act-action-1",
        action: "service.connect",
        params: {
          catalogService: {
            serviceSlug: "api-github",
            requestedScopes: ["repo"],
          },
        },
        ...overrides,
      },
    },
  };
}

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
      type: string;
      prompt: string;
      clientRequestId: string;
      sessionId?: string;
    };
    expect(Object.keys(body)).toEqual(["type", "prompt", "clientRequestId"]);
    expect(body.type).toBe("text");
    expect(body.prompt).toBe("Hello there");
    expect(body.clientRequestId).toMatch(
      /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/,
    );
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
            clientRequestId: string;
            sessionId?: string;
          },
      );
    expect(bodies).toHaveLength(2);
    expect(bodies[0]?.clientRequestId).toBe(bodies[1]?.clientRequestId);
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

describe("chat action cards", () => {
  it("creates one card and upgrades an idempotently re-emitted request", async () => {
    stubFetch(
      routeCreate,
      routeStream([
        { type: "RUN_STARTED", turnId: TURN_ID },
        actionRequestFrame({ schemaVersion: 3 }),
        actionRequestFrame(),
        {
          type: "CUSTOM",
          custom: { name: "nyxid.task.snapshot", payload: { ignored: true } },
        },
        {
          type: "RUN_FINISHED",
          runFinished: { status: "blocked" },
        },
      ]),
    );
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    const events = await collectTurn(transport, "Read my repositories");
    const starts = events.filter(
      (event) =>
        event.event === "block.started" && event.block.type === "action_card",
    );
    expect(starts).toHaveLength(1);
    const upgrade = events.find(
      (event) =>
        event.event === "block.updated" &&
        "status" in event.patch &&
        event.patch.status === "pending",
    );
    expect(upgrade).toBeDefined();

    const history = await transport.getHistory(CONVERSATION_ID);
    const cards = history.messages
      .flatMap((message) => message.blocks)
      .filter((block) => block.type === "action_card");
    expect(cards).toHaveLength(1);
    expect(cards[0]).toMatchObject({
      action_request_id: "act-action-1",
      origin_turn_id: TURN_ID,
      status: "pending",
      params: {
        variant: "catalog",
        service_slug: "api-github",
        requested_scopes: ["repo"],
      },
    });
    expect(events.at(-1)).toMatchObject({ status: "blocked" });
  });

  it("posts the exact action.continue body and streams the follow-up", async () => {
    const fetchMock = stubFetch(routeCreate, (url, init) => {
      if (!url.endsWith("/stream") || init?.method !== "POST") return undefined;
      const body = JSON.parse(String(init.body)) as { readonly type: string };
      if (body.type === "text") {
        return sseResponse([
          { type: "RUN_STARTED", turnId: TURN_ID },
          actionRequestFrame(),
          { type: "RUN_FINISHED", runFinished: { status: "blocked" } },
        ]);
      }
      return sseResponse([
        { type: "RUN_STARTED", turnId: "turn-action-follow-up" },
        {
          type: "TEXT_MESSAGE_START",
          textMessageStart: { messageId: "m-action-follow-up" },
        },
        {
          type: "TEXT_MESSAGE_CONTENT",
          textMessageContent: { delta: "GitHub is connected." },
        },
        { type: "TEXT_MESSAGE_END" },
        { type: "RUN_FINISHED" },
      ]);
    });
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();
    await collectTurn(transport, "Read my repositories");

    const events: TurnEvent[] = [];
    await new Promise<void>((resolve) => {
      const handle = transport.continueActions(
        CONVERSATION_ID,
        TURN_ID,
        [
          {
            actionRequestId: "act-action-1",
            originTurnId: TURN_ID,
            disposition: "completed",
            resource: {
              userService: {
                userServiceId: "00000000-0000-4000-8000-000000000123",
              },
            },
          },
        ],
        (event) => {
          events.push(event);
          if (event.event === "turn.completed") resolve();
        },
      );
      expect(handle).not.toBeNull();
    });

    const actionCall = fetchMock.mock.calls.find(([, init]) => {
      const body = JSON.parse(
        String((init as RequestInit | undefined)?.body),
      ) as {
        readonly type?: string;
      };
      return body.type === "action.continue";
    });
    const actionBody = JSON.parse(
      String((actionCall?.[1] as RequestInit | undefined)?.body),
    ) as Record<string, unknown>;
    expect(Object.keys(actionBody)).toEqual([
      "type",
      "clientRequestId",
      "originTurnId",
      "actions",
    ]);
    expect(actionBody).toMatchObject({
      type: "action.continue",
      originTurnId: TURN_ID,
      actions: [
        {
          actionRequestId: "act-action-1",
          originTurnId: TURN_ID,
          disposition: "completed",
          resource: {
            userService: {
              userServiceId: "00000000-0000-4000-8000-000000000123",
            },
          },
        },
      ],
    });
    expect(actionBody).not.toHaveProperty("prompt");
    expect(actionBody).not.toHaveProperty("inputParts");
    expect(actionBody).not.toHaveProperty("sessionId");
    expect(events.at(-1)).toMatchObject({
      event: "turn.completed",
      turn_id: "turn-action-follow-up",
      status: "completed",
    });
  });

  it("batches reports that resolve during the active origin turn", async () => {
    let originFinished = false;
    const actionBodies: Array<{ readonly actions: readonly unknown[] }> = [];
    stubFetch(routeCreate, (url, init) => {
      if (!url.endsWith("/stream") || init?.method !== "POST") return undefined;
      const body = JSON.parse(String(init.body)) as {
        readonly type: string;
        readonly actions?: readonly unknown[];
      };
      if (body.type === "text") {
        return sseResponse([
          { type: "RUN_STARTED", turnId: TURN_ID },
          actionRequestFrame(),
          actionRequestFrame({ actionRequestId: "act-action-2" }),
          { type: "RUN_FINISHED", runFinished: { status: "blocked" } },
        ]);
      }
      expect(originFinished).toBe(true);
      actionBodies.push({ actions: body.actions ?? [] });
      return sseResponse([
        { type: "RUN_STARTED", turnId: "turn-batched-action" },
        { type: "RUN_FINISHED" },
      ]);
    });
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();

    const resolvedIds = new Set<string>();
    await new Promise<void>((resolve) => {
      transport.sendMessage(CONVERSATION_ID, "Connect both", (event) => {
        if (
          event.event === "block.started" &&
          event.block.type === "action_card"
        ) {
          const id = event.block.action_request_id;
          if (!resolvedIds.has(id)) {
            resolvedIds.add(id);
            expect(
              transport.continueActions(
                CONVERSATION_ID,
                TURN_ID,
                [
                  {
                    actionRequestId: id,
                    originTurnId: TURN_ID,
                    disposition: "declined",
                  },
                ],
                (continuationEvent) => {
                  if (continuationEvent.event === "turn.completed") resolve();
                },
              ),
            ).toBeNull();
          }
        }
        if (event.event === "turn.completed") originFinished = true;
      });
    });

    expect(actionBodies).toHaveLength(1);
    expect(actionBodies[0]?.actions).toHaveLength(2);
  });

  it("reuses the continuation clientRequestId for automatic delivery retry", async () => {
    let actionAttempts = 0;
    const actionRequestIds: string[] = [];
    stubFetch(routeCreate, (url, init) => {
      if (!url.endsWith("/stream") || init?.method !== "POST") return undefined;
      const body = JSON.parse(String(init.body)) as {
        readonly type: string;
        readonly clientRequestId: string;
      };
      if (body.type === "text") {
        return sseResponse([
          { type: "RUN_STARTED", turnId: TURN_ID },
          actionRequestFrame(),
          { type: "RUN_FINISHED", runFinished: { status: "blocked" } },
        ]);
      }
      actionAttempts += 1;
      actionRequestIds.push(body.clientRequestId);
      return actionAttempts === 1
        ? jsonResponse({ message: "try again" }, 503)
        : sseResponse([
            { type: "RUN_STARTED", turnId: "turn-action-retry" },
            { type: "RUN_FINISHED" },
          ]);
    });
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();
    await collectTurn(transport, "Connect GitHub");

    await new Promise<void>((resolve) => {
      transport.continueActions(
        CONVERSATION_ID,
        TURN_ID,
        [
          {
            actionRequestId: "act-action-1",
            originTurnId: TURN_ID,
            disposition: "declined",
          },
        ],
        (event) => {
          if (event.event === "turn.completed") resolve();
        },
      );
    });

    expect(actionRequestIds).toHaveLength(2);
    expect(actionRequestIds[0]).toBe(actionRequestIds[1]);
  });

  it("keeps a rejected report queued and retries after the next idle turn", async () => {
    let textTurns = 0;
    let actionAttempts = 0;
    const actionRequestIds: string[] = [];
    stubFetch(routeCreate, (url, init) => {
      if (!url.endsWith("/stream") || init?.method !== "POST") return undefined;
      const body = JSON.parse(String(init.body)) as {
        readonly type: string;
        readonly clientRequestId: string;
      };
      if (body.type === "text") {
        textTurns += 1;
        return textTurns === 1
          ? sseResponse([
              { type: "RUN_STARTED", turnId: TURN_ID },
              actionRequestFrame(),
              { type: "RUN_FINISHED", runFinished: { status: "blocked" } },
            ])
          : sseResponse([
              { type: "RUN_STARTED", turnId: "turn-user-after-rejection" },
              { type: "RUN_FINISHED" },
            ]);
      }
      actionAttempts += 1;
      actionRequestIds.push(body.clientRequestId);
      return actionAttempts === 1
        ? sseResponse([
            { type: "RUN_STARTED", turnId: "turn-rejected-action" },
            {
              type: "RUN_ERROR",
              runError: {
                code: "NYXID_ACTION_CONTINUATION_ACTIVE_TURN",
                message: "Another conversation turn is active.",
              },
            },
          ])
        : sseResponse([
            { type: "RUN_STARTED", turnId: "turn-retried-action" },
            { type: "RUN_FINISHED" },
          ]);
    });
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();
    await collectTurn(transport, "Connect GitHub");

    let resolveRejected: () => void = () => undefined;
    let resolveRetried: () => void = () => undefined;
    const rejected = new Promise<void>((resolve) => {
      resolveRejected = resolve;
    });
    const retried = new Promise<void>((resolve) => {
      resolveRetried = resolve;
    });
    transport.continueActions(
      CONVERSATION_ID,
      TURN_ID,
      [
        {
          actionRequestId: "act-action-1",
          originTurnId: TURN_ID,
          disposition: "declined",
        },
      ],
      (event) => {
        if (event.event !== "turn.completed") return;
        if (event.status === "failed") resolveRejected();
        if (event.status === "completed") resolveRetried();
      },
    );
    await rejected;
    expect(actionAttempts).toBe(1);

    await collectTurn(transport, "Continue when idle");
    await retried;

    expect(actionAttempts).toBe(2);
    expect(actionRequestIds[0]).toBe(actionRequestIds[1]);
    const history = await transport.getHistory(CONVERSATION_ID);
    const card = history.messages
      .flatMap((message) => message.blocks)
      .find((block) => block.type === "action_card");
    expect(card).toMatchObject({ status: "declined" });
  });

  // Aevatar publishes a rejected continuation admission to the *origin* turn's
  // session (`nyxid.continuation.changed`), never as a reason code on the
  // continuation stream. The client therefore only ever observes a generic
  // terminal error — the report must still survive it.
  it("requeues a report after a generic continuation stream error", async () => {
    let textTurns = 0;
    let actionAttempts = 0;
    const actionRequestIds: string[] = [];
    stubFetch(routeCreate, (url, init) => {
      if (!url.endsWith("/stream") || init?.method !== "POST") return undefined;
      const body = JSON.parse(String(init.body)) as {
        readonly type: string;
        readonly clientRequestId: string;
      };
      if (body.type === "text") {
        textTurns += 1;
        return textTurns === 1
          ? sseResponse([
              { type: "RUN_STARTED", turnId: TURN_ID },
              actionRequestFrame(),
              { type: "RUN_FINISHED", runFinished: { status: "blocked" } },
            ])
          : sseResponse([
              { type: "RUN_STARTED", turnId: "turn-user-after-error" },
              { type: "RUN_FINISHED" },
            ]);
      }
      actionAttempts += 1;
      actionRequestIds.push(body.clientRequestId);
      return actionAttempts === 1
        ? sseResponse([
            { type: "RUN_STARTED", turnId: "turn-stalled-action" },
            {
              type: "RUN_ERROR",
              runError: {
                code: "STREAM_TIMEOUT",
                message: "The chat request timed out. Please try again.",
              },
            },
          ])
        : sseResponse([
            { type: "RUN_STARTED", turnId: "turn-retried-action" },
            { type: "RUN_FINISHED" },
          ]);
    });
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();
    await collectTurn(transport, "Connect GitHub");

    let resolveFailed: () => void = () => undefined;
    let resolveRetried: () => void = () => undefined;
    const failed = new Promise<void>((resolve) => {
      resolveFailed = resolve;
    });
    const retried = new Promise<void>((resolve) => {
      resolveRetried = resolve;
    });
    transport.continueActions(
      CONVERSATION_ID,
      TURN_ID,
      [
        {
          actionRequestId: "act-action-1",
          originTurnId: TURN_ID,
          disposition: "completed",
        },
      ],
      (event) => {
        if (event.event !== "turn.completed") return;
        if (event.status === "failed") resolveFailed();
        if (event.status === "completed") resolveRetried();
      },
    );
    await failed;
    expect(actionAttempts).toBe(1);

    await collectTurn(transport, "Anything else?");
    await retried;

    expect(actionAttempts).toBe(2);
    expect(actionRequestIds[0]).toBe(actionRequestIds[1]);
  });

  it("requeues a report when the continuation stalls into the watchdog", async () => {
    vi.useFakeTimers();
    try {
      const encoder = new TextEncoder();
      let textTurns = 0;
      let actionAttempts = 0;
      const actionRequestIds: string[] = [];
      // RUN_STARTED then silence: exactly what a rejected continuation looks
      // like on the wire once the server's keepalives are filtered out.
      const stalledStream = new ReadableStream<Uint8Array>({
        start(controller) {
          controller.enqueue(
            encoder.encode(
              `data: ${JSON.stringify({
                type: "RUN_STARTED",
                turnId: "turn-stalled-action",
              })}\n\n`,
            ),
          );
        },
      });
      stubFetch(routeCreate, (url, init) => {
        if (!url.endsWith("/stream") || init?.method !== "POST")
          return undefined;
        const body = JSON.parse(String(init.body)) as {
          readonly type: string;
          readonly clientRequestId: string;
        };
        if (body.type === "text") {
          textTurns += 1;
          return textTurns === 1
            ? sseResponse([
                { type: "RUN_STARTED", turnId: TURN_ID },
                actionRequestFrame(),
                { type: "RUN_FINISHED", runFinished: { status: "blocked" } },
              ])
            : sseResponse([
                { type: "RUN_STARTED", turnId: "turn-user-after-stall" },
                { type: "RUN_FINISHED" },
              ]);
        }
        actionAttempts += 1;
        actionRequestIds.push(body.clientRequestId);
        return actionAttempts === 1
          ? new Response(stalledStream, {
              status: 200,
              headers: { "Content-Type": "text/event-stream" },
            })
          : sseResponse([
              { type: "RUN_STARTED", turnId: "turn-retried-action" },
              { type: "RUN_FINISHED" },
            ]);
      });
      const transport = new AevatarAssistantTransport();
      await transport.createConversation();
      await collectTurn(transport, "Connect GitHub");

      let resolveRetried: () => void = () => undefined;
      const retried = new Promise<void>((resolve) => {
        resolveRetried = resolve;
      });
      transport.continueActions(
        CONVERSATION_ID,
        TURN_ID,
        [
          {
            actionRequestId: "act-action-1",
            originTurnId: TURN_ID,
            disposition: "completed",
          },
        ],
        (event) => {
          if (event.event === "turn.completed" && event.status === "completed") {
            resolveRetried();
          }
        },
      );
      await vi.advanceTimersByTimeAsync(120_001);
      expect(actionAttempts).toBe(1);

      const idle = new Promise<void>((resolve) => {
        transport.sendMessage(CONVERSATION_ID, "Anything else?", (event) => {
          if (event.event === "turn.completed") resolve();
        });
      });
      await vi.advanceTimersByTimeAsync(1);
      await idle;
      await retried;

      expect(actionAttempts).toBe(2);
      expect(actionRequestIds[0]).toBe(actionRequestIds[1]);
    } finally {
      vi.useRealTimers();
    }
  });

  it("downgrades a re-emitted card the client can no longer service", async () => {
    stubFetch(
      routeCreate,
      routeStream([
        { type: "RUN_STARTED", turnId: TURN_ID },
        actionRequestFrame(),
        // Aevatar rolled forward to a schema this build cannot service.
        actionRequestFrame({ schemaVersion: 5 }),
        { type: "RUN_FINISHED", runFinished: { status: "blocked" } },
      ]),
    );
    const transport = new AevatarAssistantTransport();
    await transport.createConversation();
    await collectTurn(transport, "Read my repositories");

    const history = await transport.getHistory(CONVERSATION_ID);
    const cards = history.messages
      .flatMap((message) => message.blocks)
      .filter((block) => block.type === "action_card");
    expect(cards).toHaveLength(1);
    expect(cards[0]).toMatchObject({ status: "unsupported" });
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
