import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  AevatarAssistantTransport,
  redactDisplayText,
  summarizeToolResult,
} from "@/lib/assistant/aevatar-transport";
import {
  chatStreamClient,
  type ChatStreamCompletionResult,
  type ChatStreamHeadersResult,
  type ChatStreamRequest,
  type ChatStreamRequestHandle,
} from "@/lib/assistant/chat-stream-worker-client";
import {
  AssistantConversationNotFoundError,
  AssistantTurnActiveError,
} from "@/lib/assistant/errors";
import type { ChatStreamFrame } from "@/lib/assistant/chat-stream-worker-protocol";
import { selectAssistantTransportKind } from "@/lib/assistant/transport";
import capturedHistory from "@/lib/assistant/__fixtures__/aevatar-chat-history.json";
import capturedStream from "@/lib/assistant/__fixtures__/aevatar-nyxid-chat-stream.sse?raw";
import { useAuthStore } from "@/stores/auth-store";
import {
  adoptReceiptIdentity,
  deleteReceipt,
  findReceiptByPlaceholder,
  listDeletionIntents,
  recordCreateReceipt,
  recordDeletionIntent,
  resetAssistantReceiptStoreForTests,
} from "@/stores/assistant-receipt-store";
import type { User } from "@/types/api";
import type {
  ActionCardContentBlock,
  AssistantMessage,
  ContentBlock,
  Conversation,
  TurnEvent,
  TurnReducerState,
} from "@/types/assistant";

const USER_ID = "add69059-bece-4f0e-9559-99cfd10b47eb";
const CONVERSATION_ID = "nyxid-chat-f8369965a444433f92ec50e67ad8ee52";
const TURN_ID = "turn-server-owned-1";
// NyxID's own assistant mount. No scope segment: the server derives the
// aevatar scope from the session user (PRD decision 4).
const ASSISTANT_BASE = "/api/v1/assistant";
const TYPED_COMMAND_URL = `${ASSISTANT_BASE}/chat`;

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

function legacyTypedCommandUrl(
  url: string,
  init: RequestInit | undefined,
): string | null {
  if (url !== TYPED_COMMAND_URL || init?.method !== "POST") return null;
  const body = jsonRequestBody(init);
  const conversationId =
    typeof body["conversationId"] === "string" ? body["conversationId"] : null;
  const turnId = typeof body["turnId"] === "string" ? body["turnId"] : null;
  const taskId = typeof body["taskId"] === "string" ? body["taskId"] : null;
  const stepId = typeof body["stepId"] === "string" ? body["stepId"] : null;

  if (body["type"] === "text" || body["type"] === "action.continue") {
    return conversationId
      ? `${ASSISTANT_BASE}/conversations/${conversationId}/stream`
      : null;
  }
  if (body["type"] === "approval.resolve") {
    return conversationId
      ? `${ASSISTANT_BASE}/conversations/${conversationId}/approve`
      : null;
  }
  if (body["type"] === "task.stop") {
    return conversationId
      ? `${ASSISTANT_BASE}/conversations/${conversationId}/stop`
      : null;
  }
  if (body["type"] === "task.steer") {
    return conversationId
      ? `${ASSISTANT_BASE}/conversations/${conversationId}/steer`
      : null;
  }
  if (body["type"] === "step.retry") {
    return conversationId && turnId && taskId && stepId
      ? `${ASSISTANT_BASE}/conversations/${conversationId}/turns/${turnId}/steps/${stepId}/retry`
      : null;
  }
  if (body["type"] === "step.skip") {
    return conversationId && turnId && taskId && stepId
      ? `${ASSISTANT_BASE}/conversations/${conversationId}/turns/${turnId}/steps/${stepId}/skip`
      : null;
  }
  return null;
}

function compatibleAssistantUrl(
  url: string,
  init: RequestInit | undefined,
): string {
  return legacyTypedCommandUrl(url, init) ?? url;
}

function stubFetch(...routes: FetchRoute[]): ReturnType<typeof vi.fn> {
  const mock = vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    const legacyUrl = legacyTypedCommandUrl(url, init);
    for (const route of routes) {
      const response =
        route(url, init) ?? (legacyUrl ? route(legacyUrl, init) : undefined);
      if (response) return Promise.resolve(response);
    }
    return Promise.resolve(
      jsonResponse({ error: "not_found", error_code: -1, message: "404" }, 404),
    );
  });
  vi.stubGlobal("fetch", mock);
  return mock;
}

function jsonRequestBody(
  init: RequestInit | undefined,
): Record<string, unknown> {
  return JSON.parse(String(init?.body ?? "{}")) as Record<string, unknown>;
}

function isTypedCommandRequest(
  url: string,
  init: RequestInit | undefined,
  type?: string,
): boolean {
  if (url !== TYPED_COMMAND_URL || init?.method !== "POST") return false;
  if (!type) return true;
  return jsonRequestBody(init)["type"] === type;
}

type ChatStreamRoute = (request: ChatStreamRequest) =>
  | {
      readonly headers?: ChatStreamHeadersResult;
      readonly frames?: readonly ChatStreamFrame[];
      readonly completion?: ChatStreamCompletionResult;
      readonly onCancel?: () => void;
    }
  | undefined;

function mockChatStreams(...routes: readonly ChatStreamRoute[]) {
  return vi
    .spyOn(chatStreamClient, "start")
    .mockImplementation((request): ChatStreamRequestHandle => {
      const legacyUrl = legacyTypedCommandUrl(request.url, {
        method: "POST",
        body: request.bodyText,
      });
      const legacyRequest = legacyUrl ? { ...request, url: legacyUrl } : null;
      let matched: ReturnType<ChatStreamRoute> | undefined;
      for (const route of routes) {
        matched =
          route(request) ?? (legacyRequest ? route(legacyRequest) : undefined);
        if (matched) break;
      }
      if (!matched) {
        throw new Error(`Unhandled chat stream request: ${request.url}`);
      }

      let cancelled = false;
      const headerResult: ChatStreamHeadersResult = matched.headers ?? {
        kind: "response",
        status: 200,
        contentType: "text/event-stream",
      };
      const headers = Promise.resolve(headerResult);
      const completion = Promise.resolve().then(async () => {
        const headerResult = await headers;
        if (cancelled) return { kind: "cancelled" } as const;
        if (headerResult.kind !== "response") {
          return (matched.completion ??
            headerResult) as ChatStreamCompletionResult;
        }
        if (matched.frames && matched.frames.length > 0) {
          request.onFrames(matched.frames);
        }
        return matched.completion ?? ({ kind: "complete" } as const);
      });

      return {
        headers,
        completion,
        cancel() {
          cancelled = true;
          matched?.onCancel?.();
        },
      };
    });
}

// `createConversation` is client-local and its first turn now creates the
// typed actor through `/assistant/chat`. Existing actor conversations these
// AG-UI tests exercise are seeded the way they arrive after reload: through
// the Chat History index. The
// helper stubs one list fetch, seeds `CONVERSATION_ID`, then restores the
// test's own fetch stub and the list-fetch throttle.
async function seedActorConversation(
  transport: AevatarAssistantTransport,
): Promise<Conversation> {
  const active = globalThis.fetch;
  vi.stubGlobal("fetch", (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    if (
      url === `${ASSISTANT_BASE}/conversations` &&
      (init?.method ?? "GET") === "GET"
    ) {
      return Promise.resolve(
        jsonResponse({
          conversations: [
            {
              id: CONVERSATION_ID,
              title: "Seeded conversation",
              updatedAt: "2026-07-29T00:00:00.000Z",
            },
          ],
        }),
      );
    }
    return active(input, init);
  });
  try {
    // The seed must reach the wire even when the test already listed
    // within the throttle window.
    (transport as unknown as { listFetchedAt: number }).listFetchedAt = 0;
    const conversations = await transport.listConversations();
    const seeded = conversations.find((c) => c.id === CONVERSATION_ID);
    if (!seeded) throw new Error("seed conversation did not merge");
    return seeded;
  } finally {
    vi.stubGlobal("fetch", active);
    // Reset the list throttle so tests that assert list behavior still
    // reach their own stubs.
    (transport as unknown as { listFetchedAt: number }).listFetchedAt = 0;
  }
}

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

async function actionCardsOf(
  transport: AevatarAssistantTransport,
): Promise<ActionCardContentBlock[]> {
  const history = await transport.getHistory(CONVERSATION_ID);
  return history.messages
    .flatMap((message) => message.blocks)
    .filter(
      (block): block is ActionCardContentBlock => block.type === "action_card",
    );
}

function twoTurnStub(
  second: Record<string, unknown>,
): ReturnType<typeof vi.fn> {
  let textTurns = 0;
  return stubFetch((url, init) => {
    if (!url.endsWith("/stream") || init?.method !== "POST") return undefined;
    const body = JSON.parse(String(init.body)) as { readonly type: string };
    if (body.type !== "text") {
      return sseResponse([
        { type: "RUN_STARTED", turnId: "turn-continuation" },
        { type: "RUN_FINISHED" },
      ]);
    }
    textTurns += 1;
    return sseResponse([
      { type: "RUN_STARTED", turnId: `${TURN_ID}-${textTurns}` },
      textTurns === 1 ? actionRequestFrame() : second,
      { type: "RUN_FINISHED", runFinished: { status: "blocked" } },
    ]);
  });
}

function perTurnStub(
  frames: readonly Record<string, unknown>[],
): ReturnType<typeof vi.fn> {
  let textTurns = 0;
  return stubFetch((url, init) => {
    if (!url.endsWith("/stream") || init?.method !== "POST") return undefined;
    const body = JSON.parse(String(init.body)) as { readonly type: string };
    if (body.type !== "text") {
      return sseResponse([
        { type: "RUN_STARTED", turnId: "turn-continuation" },
        { type: "RUN_FINISHED" },
      ]);
    }
    const frame = frames[textTurns] ?? frames[frames.length - 1];
    textTurns += 1;
    return sseResponse([
      { type: "RUN_STARTED", turnId: `${TURN_ID}-${textTurns}` },
      frame,
      { type: "RUN_FINISHED", runFinished: { status: "blocked" } },
    ]);
  });
}

beforeEach(() => {
  localStorage.clear();
  resetAssistantReceiptStoreForTests();
  useAuthStore.getState().setUser({ id: USER_ID } as User);
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  useAuthStore.getState().setUser(null);
});

describe("AevatarAssistantTransport", () => {
  it("adapts the observed AG-UI stream into the PRD turn-event sequence", async () => {
    stubFetch(routeStream(OBSERVED_FRAMES));
    const transport = new AevatarAssistantTransport();
    await seedActorConversation(transport);

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
    stubFetch(routeStream(OBSERVED_FRAMES), routeHistory([]));
    const transport = new AevatarAssistantTransport();
    await seedActorConversation(transport);
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
    stubFetch(routeStream(OBSERVED_FRAMES));
    const transport = new AevatarAssistantTransport();
    await seedActorConversation(transport);

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

  it("requests assistant resources as JSON", async () => {
    const fetchMock = stubFetch(
      (url, init) =>
        url === `${ASSISTANT_BASE}/conversations` &&
        (init?.method ?? "GET") === "GET"
          ? jsonResponse({ conversations: [] })
          : undefined,
      routeHistory([]),
    );
    const transport = new AevatarAssistantTransport();

    await transport.listConversations();
    await transport.getHistory(CONVERSATION_ID);

    const getCalls = fetchMock.mock.calls.filter(
      ([, init]) => (init?.method ?? "GET") === "GET",
    );
    expect(getCalls).toHaveLength(2);
    for (const [, init] of getCalls) {
      expect(init?.headers).toMatchObject({ Accept: "application/json" });
    }
  });

  it("keeps a streaming conversation's live title over stale index metadata", async () => {
    stubFetch(routeStream(OBSERVED_FRAMES));
    const transport = new AevatarAssistantTransport();
    await seedActorConversation(transport);
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
    stubFetch(routeStream(OBSERVED_FRAMES));
    const transport = new AevatarAssistantTransport();
    await seedActorConversation(transport);

    const first = collectTurn(transport, "First message");
    expect(() => {
      transport.sendMessage(CONVERSATION_ID, "Second message", () => {});
    }).toThrow(AssistantTurnActiveError);
    await first;
  });

  it("maps RUN_ERROR to a failed turn", async () => {
    stubFetch(
      routeStream([
        { type: "RUN_STARTED", turnId: TURN_ID },
        {
          type: "RUN_ERROR",
          runError: { code: "upstream_timeout", message: "Model timed out" },
        },
      ]),
    );
    const transport = new AevatarAssistantTransport();
    await seedActorConversation(transport);

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
    stubFetch((url, init) =>
      url.endsWith("/stream") && init?.method === "POST"
        ? jsonResponse({ error: "turn_active" }, 409)
        : undefined,
    );
    const transport = new AevatarAssistantTransport();
    await seedActorConversation(transport);

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
    stubFetch((url, init) =>
      url.endsWith("/stream") && init?.method === "POST"
        ? jsonResponse(
            { code: "UPSTREAM_TIMEOUT", message: "Aevatar timed out." },
            502,
          )
        : undefined,
    );
    const transport = new AevatarAssistantTransport();
    await seedActorConversation(transport);

    const events = await collectTurn(transport, "Hello");

    const terminal = events[events.length - 1];
    const error =
      terminal?.event === "turn.completed" ? terminal.error : undefined;
    expect(error?.code).toBe("UPSTREAM_TIMEOUT");
    expect(error?.message).toBe("Aevatar timed out.");
  });

  it("gives a stream 401 an auth-specific message, not a bare status", async () => {
    stubFetch((url, init) =>
      url.endsWith("/stream") && init?.method === "POST"
        ? jsonResponse({ error: "unauthorized" }, 401)
        : undefined,
    );
    const transport = new AevatarAssistantTransport();
    await seedActorConversation(transport);

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
      routeStream([OBSERVED_FRAMES[0], OBSERVED_FRAMES[1], OBSERVED_FRAMES[2]]),
    );
    const transport = new AevatarAssistantTransport();
    await seedActorConversation(transport);

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
    stubFetch((url, init) =>
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
    await seedActorConversation(transport);

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
    stubFetch((url, init) =>
      url.endsWith("/stream") && init?.method === "POST"
        ? new Response(body, {
            status: 200,
            headers: { "Content-Type": "text/event-stream" },
          })
        : undefined,
    );
    const transport = new AevatarAssistantTransport();
    await seedActorConversation(transport);

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
    stubFetch((url, init) =>
      url.endsWith("/stream") && init?.method === "POST"
        ? new Response(openStream, {
            status: 200,
            headers: { "Content-Type": "text/event-stream" },
          })
        : undefined,
    );
    const transport = new AevatarAssistantTransport();
    await seedActorConversation(transport);

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
    await seedActorConversation(transport);

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
    const fetchMock = stubFetch((url, init) =>
      url.endsWith("/stream") && init?.method === "POST"
        ? new Response(silentStream, {
            status: 200,
            headers: { "Content-Type": "text/event-stream" },
          })
        : undefined,
    );
    const transport = new AevatarAssistantTransport();
    await seedActorConversation(transport);

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
    await seedActorConversation(transport);

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
        const url = compatibleAssistantUrl(String(input), init);
        if (
          url === `${ASSISTANT_BASE}/conversations` &&
          init?.method === "POST"
        ) {
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
    await seedActorConversation(transport);

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
        const url = compatibleAssistantUrl(String(input), init);
        if (
          url === `${ASSISTANT_BASE}/conversations` &&
          init?.method === "POST"
        ) {
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
    await seedActorConversation(transport);

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
        const url = compatibleAssistantUrl(String(input), init);
        if (
          url === `${ASSISTANT_BASE}/conversations` &&
          init?.method === "POST"
        ) {
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
    await seedActorConversation(transport);

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
        const url = compatibleAssistantUrl(String(input), init);
        if (
          url === `${ASSISTANT_BASE}/conversations` &&
          init?.method === "POST"
        ) {
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
    await seedActorConversation(transport);

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
    ).toThrow(AssistantConversationNotFoundError);
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
        const url = compatibleAssistantUrl(String(input), init);
        if (
          url === `${ASSISTANT_BASE}/conversations` &&
          init?.method === "POST"
        ) {
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
    await seedActorConversation(transport);

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
      await seedActorConversation(transport);

      const firstOutcome = transport.deleteConversation(CONVERSATION_ID).then(
        () => "resolved",
        () => "rejected",
      );
      const secondOutcome = transport.deleteConversation(CONVERSATION_ID).then(
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
      ).toThrow(AssistantConversationNotFoundError);
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
    await seedActorConversation(transport);
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
        const url = compatibleAssistantUrl(String(input), init);
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
    await seedActorConversation(transport);

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
    ).toThrow(AssistantConversationNotFoundError);
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
        const url = compatibleAssistantUrl(String(input), init);
        if (
          url === `${ASSISTANT_BASE}/conversations` &&
          init?.method === "POST"
        ) {
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
    await seedActorConversation(transport);

    // Turn 1 parks an actionable approval card at EOF.
    const turn1 = await collectTurn(transport, "Post the digest");
    const card = turn1.find(
      (event) =>
        event.event === "block.started" && event.block.type === "approval_card",
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
        const url = compatibleAssistantUrl(String(input), init);
        if (
          url === `${ASSISTANT_BASE}/conversations` &&
          init?.method === "POST"
        ) {
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
    await seedActorConversation(transport);

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
    await seedActorConversation(transport);

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
    }).toThrow(AssistantConversationNotFoundError);
  });

  it("never puts a scope id on the wire, even with no user in the store", async () => {
    // The server derives the aevatar scope from the verified session, so the
    // transport must not depend on the client-side user at all (PRD
    // decision 4). Regression guard against reintroducing a scope segment.
    useAuthStore.getState().setUser(null);
    const fetchMock = stubFetch(routeStream(OBSERVED_FRAMES));
    const transport = new AevatarAssistantTransport();
    await seedActorConversation(transport);

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

  it("accepts a bodyless 204 delete and drops the conversation locally", async () => {
    const routeIndex: FetchRoute = (url, init) =>
      url === `${ASSISTANT_BASE}/conversations` &&
      (init?.method ?? "GET") === "GET"
        ? jsonResponse({ conversations: [] })
        : undefined;
    const routeDelete: FetchRoute = (url, init) =>
      url === `${ASSISTANT_BASE}/conversations/${CONVERSATION_ID}` &&
      init?.method === "DELETE"
        ? new Response(null, { status: 204 })
        : undefined;
    const fetchMock = stubFetch(routeIndex, routeDelete);
    const transport = new AevatarAssistantTransport();
    await seedActorConversation(transport);
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
    stubFetch(routeIndex, routeDeleteFailure);
    const transport = new AevatarAssistantTransport();
    await seedActorConversation(transport);

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
    stubFetch(routeDelete, (url, init) =>
      url.endsWith("/stream") && init?.method === "POST"
        ? new Response(openStream, {
            status: 200,
            headers: { "Content-Type": "text/event-stream" },
          })
        : undefined,
    );
    const transport = new AevatarAssistantTransport();
    await seedActorConversation(transport);

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
    ).toThrow(AssistantConversationNotFoundError);
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
    stubFetch((url, init) =>
      url.endsWith("/stream") && init?.method === "POST"
        ? new Response(trickle, {
            status: 200,
            headers: { "Content-Type": "text/event-stream" },
          })
        : undefined,
    );
    const transport = new AevatarAssistantTransport();
    await seedActorConversation(transport);

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
    stubFetch(routeHistory({ messages: capturedHistory, stateVersion: 7 }));
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
                conversations: [{ id: CONVERSATION_ID, title: "Server title" }],
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
    const fetchMock = stubFetch(routeStream(OBSERVED_FRAMES));
    const transport = new AevatarAssistantTransport();
    await seedActorConversation(transport);

    await collectTurn(transport, "Hello there");

    const streamCall = fetchMock.mock.calls.find(([input, init]) =>
      isTypedCommandRequest(
        String(input),
        init as RequestInit | undefined,
        "text",
      ),
    ) as [string, RequestInit] | undefined;
    expect(streamCall).toBeDefined();
    const [url, init] = streamCall ?? ["", {}];
    // NyxID's own route: no scope segment, because the server derives the
    // aevatar scope from the verified session. The endpoint still 415s
    // without the explicit JSON content type.
    expect(url).toBe(TYPED_COMMAND_URL);
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
      conversationId: string;
      prompt: string;
      clientRequestId: string;
      sessionId?: string;
    };
    expect(Object.keys(body)).toEqual([
      "type",
      "conversationId",
      "prompt",
      "clientRequestId",
    ]);
    expect(body.type).toBe("text");
    expect(body.conversationId).toBe(CONVERSATION_ID);
    expect(body.prompt).toBe("Hello there");
    expect(body.clientRequestId).toMatch(
      /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/,
    );
    expect(Object.keys(body).sort()).toEqual([
      "clientRequestId",
      "conversationId",
      "prompt",
      "type",
    ]);
    expect(body.sessionId).toBeUndefined();
  });

  it("uses a new clientRequestId for each logical turn across reprojection", async () => {
    const fetchMock = stubFetch(
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
    await seedActorConversation(transport);

    await collectTurn(transport, "First turn");
    // Post-turn reprojection: server history (equal length) replaces the
    // local mirror; the list merge rebuilds the conversation row.
    await transport.getHistory(CONVERSATION_ID);
    await transport.listConversations();
    await collectTurn(transport, "Second turn");

    const clientRequestIds = fetchMock.mock.calls
      .filter(([input, init]) =>
        isTypedCommandRequest(
          String(input),
          init as RequestInit | undefined,
          "text",
        ),
      )
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
        const url = compatibleAssistantUrl(String(input), init);
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
    await seedActorConversation(transport);

    const events = await collectTurn(transport, "Retry this delivery");

    expect(events.at(-1)).toMatchObject({
      event: "turn.completed",
      turn_id: TURN_ID,
      status: "completed",
    });
    const bodies = fetchMock.mock.calls
      .filter(([input, init]) =>
        isTypedCommandRequest(
          String(input),
          init as RequestInit | undefined,
          "text",
        ),
      )
      .map(
        ([, init]) =>
          JSON.parse(String(init?.body)) as {
            prompt: string;
            conversationId: string;
            clientRequestId: string;
            type: string;
            sessionId?: string;
          },
      );
    expect(bodies).toHaveLength(2);
    expect(bodies[0]?.clientRequestId).toBe(bodies[1]?.clientRequestId);
    expect(bodies[0]).toEqual({
      conversationId: CONVERSATION_ID,
      prompt: "Retry this delivery",
      clientRequestId: bodies[0]?.clientRequestId,
      type: "text",
    });
    expect(bodies[1]).toEqual(bodies[0]);
    expect(bodies[0]?.sessionId).toBeUndefined();
  });

  it("retries a successful stream response that has no body", async () => {
    let streamAttempts = 0;
    const fetchMock = stubFetch((url, init) => {
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
    await seedActorConversation(transport);

    const events = await collectTurn(transport, "Retry the empty delivery");

    expect(events.at(-1)).toMatchObject({
      event: "turn.completed",
      turn_id: TURN_ID,
      status: "completed",
    });
    const requestIds = fetchMock.mock.calls
      .filter(([input, init]) =>
        isTypedCommandRequest(
          String(input),
          init as RequestInit | undefined,
          "text",
        ),
      )
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
    await seedActorConversation(transport);

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
    await seedActorConversation(transport);

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
    await seedActorConversation(transport);

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
    await seedActorConversation(transport);

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
    await seedActorConversation(transport);

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
    const fetchMock = stubFetch((url, init) => {
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
    await seedActorConversation(transport);

    const blocked = await collectTurn(transport, "Read a private repository");
    const completed = await collectTurn(transport, "Continue after reconnect");

    expect(blocked.at(-1)).toMatchObject({ status: "blocked" });
    expect(completed.at(-1)).toMatchObject({
      status: "completed",
      turn_id: "turn-server-owned-2",
    });
    const requestIds = fetchMock.mock.calls
      .filter(([input, init]) =>
        isTypedCommandRequest(
          String(input),
          init as RequestInit | undefined,
          "text",
        ),
      )
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
    await seedActorConversation(transport);

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
    await seedActorConversation(transport);
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

    const approveCall = fetchMock.mock.calls.find(([input, init]) =>
      isTypedCommandRequest(
        String(input),
        init as RequestInit | undefined,
        "approval.resolve",
      ),
    );
    const approveBody = JSON.parse(
      String((approveCall?.[1] as RequestInit | undefined)?.body),
    ) as {
      type: string;
      conversationId: string;
      clientRequestId: string;
      requestId: string;
      approved: boolean;
      sessionId?: string;
    };
    expect(approveBody.type).toBe("approval.resolve");
    expect(approveBody.conversationId).toBe(CONVERSATION_ID);
    expect(approveBody.clientRequestId).toMatch(/^[0-9a-f-]{36}$/);
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
    await seedActorConversation(transport);
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
    await seedActorConversation(transport);
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
        if (isTypedCommandRequest(String(input), init, "approval.resolve")) {
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
    await seedActorConversation(transport);

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
    await seedActorConversation(transport);

    await transport.deleteConversation(CONVERSATION_ID);

    // Stale index still returns the row — the tombstone must filter it.
    const list = await transport.listConversations();
    expect(list).toHaveLength(0);
  });

  it("settles immediately when the approve endpoint acks with JSON", async () => {
    stubFetch(
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
    await seedActorConversation(transport);
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
    await seedActorConversation(transport);
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
    await seedActorConversation(transport);
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
    await seedActorConversation(transport);

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
    await seedActorConversation(transport);

    await transport.deleteConversation(CONVERSATION_ID);

    await expect(transport.getHistory(CONVERSATION_ID)).rejects.toBeInstanceOf(
      AssistantConversationNotFoundError,
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
    await seedActorConversation(transport);
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

    await expect(pendingRead).rejects.toBeInstanceOf(
      AssistantConversationNotFoundError,
    );
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

    await expect(pendingRead).rejects.toBeInstanceOf(
      AssistantConversationNotFoundError,
    );
  });

  it("stops an approve request hung before response headers", async () => {
    // Second-pass codex P2: Stop works during the pre-header window via the
    // transport-level cancel (the caller holds no handle yet).
    stubFetch(
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
    await seedActorConversation(transport);
    await collectTurn(transport, "Do it");
    const history = await transport.getHistory(CONVERSATION_ID);
    const card = history.messages
      .flatMap((message) => message.blocks)
      .find((block) => block.type === "approval_card");

    const baseFetch = fetch;
    vi.stubGlobal(
      "fetch",
      vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
        if (isTypedCommandRequest(String(input), init, "approval.resolve")) {
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
    await seedActorConversation(transport);
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
    await seedActorConversation(transport);

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
    await seedActorConversation(transport);

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
    await seedActorConversation(transport);

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
    await seedActorConversation(transport);

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
    await seedActorConversation(transport);

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
    await seedActorConversation(transport);

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
      stubFetch((url, init) =>
        url.endsWith("/stream") && init?.method === "POST"
          ? new Response(openStream, {
              status: 200,
              headers: { "Content-Type": "text/event-stream" },
            })
          : undefined,
      );
      const transport = new AevatarAssistantTransport();
      await seedActorConversation(transport);

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
      stubFetch((url, init) =>
        url.endsWith("/stream") && init?.method === "POST"
          ? new Response(openStream, {
              status: 200,
              headers: { "Content-Type": "text/event-stream" },
            })
          : undefined,
      );
      const transport = new AevatarAssistantTransport();
      await seedActorConversation(transport);

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
  it("treats a deep-equal re-emission as an idempotent no-op", async () => {
    stubFetch(
      routeStream([
        { type: "RUN_STARTED", turnId: TURN_ID },
        actionRequestFrame(),
        actionRequestFrame(),
        {
          type: "RUN_FINISHED",
          runFinished: { status: "blocked" },
        },
      ]),
    );
    const transport = new AevatarAssistantTransport();
    await seedActorConversation(transport);

    const events = await collectTurn(transport, "Read my repositories");
    const starts = events.filter(
      (event): event is Extract<TurnEvent, { event: "block.started" }> =>
        event.event === "block.started" && event.block.type === "action_card",
    );
    expect(starts).toHaveLength(1);
    const duplicateUpdate = events.find(
      (event) =>
        event.event === "block.updated" &&
        "status" in event.patch &&
        event.block_id === starts[0]?.block.block_id,
    );
    expect(duplicateUpdate).toBeUndefined();

    const history = await transport.getHistory(CONVERSATION_ID);
    const cards = history.messages
      .flatMap((message) => message.blocks)
      .filter((block) => block.type === "action_card");
    expect(cards).toHaveLength(1);
    expect(cards[0]).toMatchObject({
      action_request_id: "act-action-1",
      origin_turn_id: TURN_ID,
      task_id: "task-action-1",
      step_id: "step-action-1",
      status: "pending",
      params: {
        variant: "catalog",
        service_slug: "api-github",
        requested_scopes: ["repo"],
      },
    });
    expect(events.at(-1)).toMatchObject({ status: "blocked" });
  });

  it("re-arms a blocked card when the assistant reissues the same request later", async () => {
    twoTurnStub(actionRequestFrame());
    const transport = new AevatarAssistantTransport();
    await seedActorConversation(transport);
    await collectTurn(transport, "Connect GitHub");

    const [card] = await actionCardsOf(transport);
    if (!card) throw new Error("no action card");
    transport.blockActionCard(
      CONVERSATION_ID,
      card.block_id,
      "Connected, but NyxID could not verify which service was created. Manage it in AI Services, then ask the assistant to request it again.",
    );
    expect((await actionCardsOf(transport))[0]?.status).toBe("blocked");

    await collectTurn(transport, "Please request it again");

    const [after] = await actionCardsOf(transport);
    expect(after).toMatchObject({
      status: "pending",
      outcome_note: "",
    });
  });

  it("keeps a blocked reissue unsupported when the exact request is no longer serviceable", async () => {
    twoTurnStub(actionRequestFrame({ schemaVersion: 5 }));
    const transport = new AevatarAssistantTransport();
    await seedActorConversation(transport);
    await collectTurn(transport, "Connect GitHub");
    const [card] = await actionCardsOf(transport);
    if (!card) throw new Error("no action card");
    transport.blockActionCard(CONVERSATION_ID, card.block_id, "no id");

    await collectTurn(transport, "Please request it again");

    expect((await actionCardsOf(transport))[0]).toMatchObject({
      status: "unsupported",
      outcome_note: "no id",
    });
  });

  it("keeps a mismatched blocked-card reissue conflicted and preserves the first request", async () => {
    twoTurnStub(
      actionRequestFrame({
        params: { catalogService: { serviceSlug: "api-lark" } },
      }),
    );
    const transport = new AevatarAssistantTransport();
    await seedActorConversation(transport);
    await collectTurn(transport, "Connect GitHub");
    const [card] = await actionCardsOf(transport);
    if (!card) throw new Error("no action card");
    transport.blockActionCard(CONVERSATION_ID, card.block_id, "no id");

    await collectTurn(transport, "Please request it again");

    expect((await actionCardsOf(transport))[0]).toMatchObject({
      status: "conflicted",
      params: { variant: "catalog", service_slug: "api-github" },
    });
  });

  it("completes the journey end-to-end after a blocked card is re-armed", async () => {
    const fetchMock = twoTurnStub(actionRequestFrame());
    const transport = new AevatarAssistantTransport();
    await seedActorConversation(transport);
    await collectTurn(transport, "Connect GitHub");
    const [card] = await actionCardsOf(transport);
    if (!card) throw new Error("no action card");
    transport.blockActionCard(CONVERSATION_ID, card.block_id, "no id");
    await collectTurn(transport, "Please request it again");
    expect((await actionCardsOf(transport))[0]?.status).toBe("pending");

    transport.setActionCardInProgress(CONVERSATION_ID, card.block_id, true);
    await new Promise<void>((resolve) => {
      transport.continueActions(
        CONVERSATION_ID,
        `${TURN_ID}-2`,
        [
          {
            actionRequestId: "act-action-1",
            originTurnId: `${TURN_ID}-2`,
            disposition: "completed",
            resource: {
              userService: {
                userServiceId: "00000000-0000-4000-8000-000000000123",
              },
            },
          },
        ],
        (event) => {
          if (event.event === "turn.completed") resolve();
        },
      );
    });

    expect((await actionCardsOf(transport))[0]?.status).toBe("completed");
    expect(
      fetchMock.mock.calls.some(([, init]) => {
        const rawBody = (init as RequestInit | undefined)?.body;
        if (!rawBody) return false;
        const body = JSON.parse(String(rawBody)) as { readonly type?: string };
        return body.type === "action.continue";
      }),
    ).toBe(true);
  });

  it("does not let a blocked card reach completed through an in_progress hop", async () => {
    const fetchMock = twoTurnStub(actionRequestFrame());
    const transport = new AevatarAssistantTransport();
    await seedActorConversation(transport);
    await collectTurn(transport, "Connect GitHub");
    const [card] = await actionCardsOf(transport);
    if (!card) throw new Error("no action card");
    transport.blockActionCard(CONVERSATION_ID, card.block_id, "no id");

    transport.setActionCardInProgress(CONVERSATION_ID, card.block_id, true);
    expect((await actionCardsOf(transport))[0]?.status).toBe("blocked");
    expect(() =>
      transport.continueActions(CONVERSATION_ID, `${TURN_ID}-1`, [
        {
          actionRequestId: "act-action-1",
          originTurnId: `${TURN_ID}-1`,
          disposition: "completed",
          resource: {
            userService: {
              userServiceId: "00000000-0000-4000-8000-000000000123",
            },
          },
        },
      ]),
    ).toThrow();
    await Promise.resolve();
    expect(
      fetchMock.mock.calls.some(([, init]) => {
        const rawBody = (init as RequestInit | undefined)?.body;
        if (!rawBody) return false;
        const body = JSON.parse(String(rawBody)) as { readonly type?: string };
        return body.type === "action.continue";
      }),
    ).toBe(false);
  });

  it("marks a same-id request with different params as conflicted and keeps the first request", async () => {
    const fetchMock = stubFetch(
      routeStream([
        { type: "RUN_STARTED", turnId: TURN_ID },
        actionRequestFrame(),
        actionRequestFrame({
          params: {
            catalogService: {
              serviceSlug: "api-lark",
              requestedScopes: ["messages:write"],
            },
          },
        }),
        { type: "RUN_FINISHED", runFinished: { status: "blocked" } },
      ]),
    );
    const transport = new AevatarAssistantTransport();
    await seedActorConversation(transport);
    await collectTurn(transport, "Connect GitHub");

    const history = await transport.getHistory(CONVERSATION_ID);
    const card = history.messages
      .flatMap((message) => message.blocks)
      .find((block) => block.type === "action_card");
    expect(card).toMatchObject({
      type: "action_card",
      status: "conflicted",
      outcome_note:
        "This action request was reissued with conflicting details. NyxID kept the first request and disabled this card.",
      params: {
        variant: "catalog",
        service_slug: "api-github",
        requested_scopes: ["repo"],
      },
    });

    expect(() =>
      transport.continueActions(CONVERSATION_ID, TURN_ID, [
        {
          actionRequestId: "act-action-1",
          originTurnId: TURN_ID,
          disposition: "declined",
        },
      ]),
    ).toThrow(
      "This action request can no longer be continued from the current card state.",
    );
    expect(
      fetchMock.mock.calls.some(([, init]) => {
        const rawBody = (init as RequestInit | undefined)?.body;
        if (!rawBody) return false;
        const body = JSON.parse(String(rawBody)) as { readonly type?: string };
        return body.type === "action.continue";
      }),
    ).toBe(false);
  });

  it("patches conflicted cards when a connected service could not be reported", async () => {
    const fetchMock = stubFetch(
      routeStream([
        { type: "RUN_STARTED", turnId: TURN_ID },
        actionRequestFrame(),
        actionRequestFrame({
          params: {
            catalogService: {
              serviceSlug: "api-lark",
              requestedScopes: ["messages:write"],
            },
          },
        }),
        { type: "RUN_FINISHED", runFinished: { status: "blocked" } },
      ]),
    );
    const transport = new AevatarAssistantTransport();
    await seedActorConversation(transport);
    await collectTurn(transport, "Connect GitHub");

    expect(() =>
      transport.continueActions(CONVERSATION_ID, TURN_ID, [
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
      ]),
    ).toThrow(
      "This action request can no longer be continued from the current card state.",
    );

    const [card] = await actionCardsOf(transport);
    expect(card).toMatchObject({
      status: "conflicted",
    });
    expect(card?.outcome_note).toContain(
      "This action request was reissued with conflicting details. NyxID kept the first request and disabled this card.",
    );
    expect(card?.outcome_note).toContain(
      "A service was connected in NyxID, but this action request could not notify the assistant. Review it in AI Services.",
    );
    expect(
      fetchMock.mock.calls.some(([, init]) => {
        const rawBody = (init as RequestInit | undefined)?.body;
        if (!rawBody) return false;
        const body = JSON.parse(String(rawBody)) as { readonly type?: string };
        return body.type === "action.continue";
      }),
    ).toBe(false);
  });

  it("accepts decline and failure reports from blocked cards", async () => {
    const fetchMock = stubFetch((url, init) => {
      if (!url.endsWith("/stream") || init?.method !== "POST") return undefined;
      const body = JSON.parse(String(init.body)) as { readonly type: string };
      if (body.type === "text") {
        return sseResponse([
          { type: "RUN_STARTED", turnId: TURN_ID },
          actionRequestFrame(),
          actionRequestFrame({ actionRequestId: "act-action-2" }),
          { type: "RUN_FINISHED", runFinished: { status: "blocked" } },
        ]);
      }
      return sseResponse([
        { type: "RUN_STARTED", turnId: "turn-blocked-resolution" },
        { type: "RUN_FINISHED" },
      ]);
    });
    const transport = new AevatarAssistantTransport();
    await seedActorConversation(transport);
    await collectTurn(transport, "Connect both services");

    for (const card of await actionCardsOf(transport)) {
      transport.blockActionCard(CONVERSATION_ID, card.block_id, "no id");
    }

    await new Promise<void>((resolve) => {
      const handle = transport.continueActions(
        CONVERSATION_ID,
        TURN_ID,
        [
          {
            actionRequestId: "act-action-1",
            originTurnId: TURN_ID,
            disposition: "failed",
          },
          {
            actionRequestId: "act-action-2",
            originTurnId: TURN_ID,
            disposition: "declined",
          },
        ],
        (event) => {
          if (event.event === "turn.completed") resolve();
        },
      );
      expect(handle).not.toBeNull();
    });

    const cards = await actionCardsOf(transport);
    expect(cards).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          action_request_id: "act-action-1",
          status: "failed",
        }),
        expect.objectContaining({
          action_request_id: "act-action-2",
          status: "declined",
        }),
      ]),
    );
    expect(
      fetchMock.mock.calls.some(([, init]) => {
        const rawBody = (init as RequestInit | undefined)?.body;
        if (!rawBody) return false;
        const body = JSON.parse(String(rawBody)) as { readonly type?: string };
        return body.type === "action.continue";
      }),
    ).toBe(true);
  });

  it("patches blocked cards before refusing a completed report that carries a resource", async () => {
    const fetchMock = stubFetch(
      routeStream([
        { type: "RUN_STARTED", turnId: TURN_ID },
        actionRequestFrame(),
        { type: "RUN_FINISHED", runFinished: { status: "blocked" } },
      ]),
    );
    const transport = new AevatarAssistantTransport();
    await seedActorConversation(transport);
    await collectTurn(transport, "Connect GitHub");

    const [card] = await actionCardsOf(transport);
    if (!card) throw new Error("no action card");
    transport.blockActionCard(CONVERSATION_ID, card.block_id, "no id");

    expect(() =>
      transport.continueActions(CONVERSATION_ID, TURN_ID, [
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
      ]),
    ).toThrow(
      "This action request can no longer be continued from the current card state.",
    );

    expect((await actionCardsOf(transport))[0]).toMatchObject({
      status: "blocked",
      outcome_note:
        "A service was connected in NyxID, but this action request could not notify the assistant. Review it in AI Services.",
    });
    expect(
      fetchMock.mock.calls.some(([, init]) => {
        const rawBody = (init as RequestInit | undefined)?.body;
        if (!rawBody) return false;
        const body = JSON.parse(String(rawBody)) as { readonly type?: string };
        return body.type === "action.continue";
      }),
    ).toBe(false);
  });

  it("posts the exact action.continue body and streams the follow-up", async () => {
    const fetchMock = stubFetch((url, init) => {
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
    await seedActorConversation(transport);
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
      "conversationId",
      "clientRequestId",
      "originTurnId",
      "actions",
    ]);
    expect(actionBody).toMatchObject({
      type: "action.continue",
      conversationId: CONVERSATION_ID,
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

    const history = await transport.getHistory(CONVERSATION_ID);
    const card = history.messages
      .flatMap((message) => message.blocks)
      .find((block) => block.type === "action_card");
    expect(card).toMatchObject({
      type: "action_card",
      status: "completed",
      outcome_note: "Reported — awaiting assistant verification.",
    });
  });

  it("batches reports that resolve during the active origin turn", async () => {
    let originFinished = false;
    const actionBodies: Array<{ readonly actions: readonly unknown[] }> = [];
    stubFetch((url, init) => {
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
    await seedActorConversation(transport);

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

  // The decline button is only disabled once the patched status has rendered,
  // so a fast double-click can reach the transport twice before React repaints.
  it("sends one report per action request when a decline is double-fired", async () => {
    const actionBodies: Array<{ readonly actions: readonly unknown[] }> = [];
    stubFetch((url, init) => {
      if (!url.endsWith("/stream") || init?.method !== "POST") return undefined;
      const body = JSON.parse(String(init.body)) as {
        readonly type: string;
        readonly actions?: readonly unknown[];
      };
      if (body.type === "text") {
        return sseResponse([
          { type: "RUN_STARTED", turnId: TURN_ID },
          actionRequestFrame(),
          { type: "RUN_FINISHED", runFinished: { status: "blocked" } },
        ]);
      }
      actionBodies.push({ actions: body.actions ?? [] });
      return sseResponse([
        { type: "RUN_STARTED", turnId: "turn-double-decline" },
        { type: "RUN_FINISHED" },
      ]);
    });
    const transport = new AevatarAssistantTransport();
    await seedActorConversation(transport);
    await collectTurn(transport, "Connect GitHub");

    const report = {
      actionRequestId: "act-action-1",
      originTurnId: TURN_ID,
      disposition: "declined",
    } as const;
    await new Promise<void>((resolve) => {
      transport.continueActions(CONVERSATION_ID, TURN_ID, [report], (event) => {
        if (event.event === "turn.completed") resolve();
      });
      // Second click lands before the first continuation settles.
      expect(
        transport.continueActions(CONVERSATION_ID, TURN_ID, [report]),
      ).toBeNull();
    });

    expect(actionBodies).toHaveLength(1);
    expect(actionBodies[0]?.actions).toHaveLength(1);
  });

  it("reuses the continuation clientRequestId for automatic delivery retry", async () => {
    let actionAttempts = 0;
    const actionRequestIds: string[] = [];
    stubFetch((url, init) => {
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
    await seedActorConversation(transport);
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

  it("requeues a continuation when a successful response is not SSE", async () => {
    let textTurns = 0;
    let actionAttempts = 0;
    const actionRequestIds: string[] = [];
    stubFetch((url, init) => {
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
              { type: "RUN_STARTED", turnId: "turn-after-json-response" },
              { type: "RUN_FINISHED" },
            ]);
      }
      actionAttempts += 1;
      actionRequestIds.push(body.clientRequestId);
      return actionAttempts === 1
        ? jsonResponse({ accepted: true })
        : sseResponse([
            { type: "RUN_STARTED", turnId: "turn-retried-after-json" },
            { type: "RUN_FINISHED" },
          ]);
    });
    const transport = new AevatarAssistantTransport();
    await seedActorConversation(transport);
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
          resource: {
            userService: {
              userServiceId: "00000000-0000-4000-8000-000000000123",
            },
          },
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
    let history = await transport.getHistory(CONVERSATION_ID);
    let card = history.messages
      .flatMap((message) => message.blocks)
      .find((block) => block.type === "action_card");
    expect(card).toMatchObject({ status: "completed" });
    expect(card?.type === "action_card" ? card.outcome_note : "").toContain(
      "has not reached the assistant",
    );
    expect(card?.type === "action_card" ? card.outcome_note : "").not.toContain(
      "assistant received",
    );

    await collectTurn(transport, "Continue when idle");
    await retried;

    expect(actionAttempts).toBe(2);
    expect(actionRequestIds[0]).toBe(actionRequestIds[1]);
    history = await transport.getHistory(CONVERSATION_ID);
    card = history.messages
      .flatMap((message) => message.blocks)
      .find((block) => block.type === "action_card");
    expect(card?.type === "action_card" ? card.outcome_note : "").toBe(
      "Reported — awaiting assistant verification.",
    );
  });

  it("keeps a rejected report queued and retries after the next idle turn", async () => {
    let textTurns = 0;
    let actionAttempts = 0;
    const actionRequestIds: string[] = [];
    stubFetch((url, init) => {
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
    await seedActorConversation(transport);
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
    stubFetch((url, init) => {
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
    await seedActorConversation(transport);
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
          resource: {
            userService: {
              userServiceId: "00000000-0000-4000-8000-000000000123",
            },
          },
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
      stubFetch((url, init) => {
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
      await seedActorConversation(transport);
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
            resource: {
              userService: {
                userServiceId: "00000000-0000-4000-8000-000000000123",
              },
            },
          },
        ],
        (event) => {
          if (
            event.event === "turn.completed" &&
            event.status === "completed"
          ) {
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
      routeStream([
        { type: "RUN_STARTED", turnId: TURN_ID },
        actionRequestFrame(),
        // Aevatar rolled forward to a schema this build cannot service.
        actionRequestFrame({ schemaVersion: 5 }),
        { type: "RUN_FINISHED", runFinished: { status: "blocked" } },
      ]),
    );
    const transport = new AevatarAssistantTransport();
    await seedActorConversation(transport);
    await collectTurn(transport, "Read my repositories");

    const history = await transport.getHistory(CONVERSATION_ID);
    const cards = history.messages
      .flatMap((message) => message.blocks)
      .filter((block) => block.type === "action_card");
    expect(cards).toHaveLength(1);
    expect(cards[0]).toMatchObject({ status: "unsupported" });
  });

  it("conflicts schema-valid requests that both normalize to unknown under one actionRequestId", async () => {
    perTurnStub([
      actionRequestFrame({
        params: {
          customService: {
            name: "Build API",
            endpointUrl: "http://build.example.test/v1",
          },
        },
      }),
      actionRequestFrame({
        params: {
          customService: {
            name: "Build API",
            endpointUrl: "https://build.example.test/v1?token=nope",
          },
        },
      }),
    ]);
    const transport = new AevatarAssistantTransport();
    await seedActorConversation(transport);

    await collectTurn(transport, "Unsupported request 1");
    expect((await actionCardsOf(transport))[0]).toMatchObject({
      status: "unsupported",
      params: { variant: "unknown" },
    });

    await collectTurn(transport, "Unsupported request 2");
    expect((await actionCardsOf(transport))[0]).toMatchObject({
      status: "conflicted",
    });
  });

  it("treats recovered payloads with stable key reordering as a no-op", async () => {
    perTurnStub([
      {
        type: "CUSTOM",
        custom: {
          name: "nyxid.action.request",
          payload: {
            schemaVersion: 4,
            actorId: CONVERSATION_ID,
            originTurnId: TURN_ID,
            actionRequestId: "act-action-1",
            action: "service.connect",
            params: {
              customService: {
                name: "Build API",
                endpointUrl: "https://build.example.test/v1",
                unexpected: { beta: 2, alpha: [1, "x", true, null] },
              },
            },
          },
        },
      },
      {
        type: "CUSTOM",
        custom: {
          name: "nyxid.action.request",
          payload: {
            action: "service.connect",
            actionRequestId: "act-action-1",
            originTurnId: TURN_ID,
            actorId: CONVERSATION_ID,
            schemaVersion: 4,
            params: {
              customService: {
                unexpected: { alpha: [1, "x", true, null], beta: 2 },
                endpointUrl: "https://build.example.test/v1",
                name: "Build API",
              },
            },
          },
        },
      },
    ]);
    const transport = new AevatarAssistantTransport();
    await seedActorConversation(transport);

    await collectTurn(transport, "Recovered request 1");
    expect((await actionCardsOf(transport))[0]?.status).toBe("unsupported");

    await collectTurn(transport, "Recovered request 2");
    expect((await actionCardsOf(transport))[0]?.status).toBe("unsupported");
  });

  it("treats requested scope array reordering as a different request", async () => {
    perTurnStub([
      actionRequestFrame({
        params: {
          catalogService: {
            serviceSlug: "api-github",
            requestedScopes: ["repo", "user"],
          },
        },
      }),
      actionRequestFrame({
        params: {
          catalogService: {
            serviceSlug: "api-github",
            requestedScopes: ["user", "repo"],
          },
        },
      }),
    ]);
    const transport = new AevatarAssistantTransport();
    await seedActorConversation(transport);

    await collectTurn(transport, "Scope order 1");
    expect((await actionCardsOf(transport))[0]?.status).toBe("pending");

    await collectTurn(transport, "Scope order 2");
    expect((await actionCardsOf(transport))[0]?.status).toBe("conflicted");
  });

  it("distinguishes recovered malformed re-emissions by their hashed original payloads", async () => {
    perTurnStub([
      actionRequestFrame({
        params: {
          customService: {
            name: "Build API",
            endpointUrl: "https://build.example.test/v1",
            unexpected: "value-1",
          },
        },
      }),
      actionRequestFrame({
        params: {
          customService: {
            name: "Build API",
            endpointUrl: "https://build.example.test/v1",
            unexpected: "value-1",
          },
        },
      }),
      actionRequestFrame({
        params: {
          customService: {
            name: "Build API",
            endpointUrl: "https://build.example.test/v1",
            unexpected: "value-3",
          },
        },
      }),
    ]);
    const transport = new AevatarAssistantTransport();
    await seedActorConversation(transport);

    await collectTurn(transport, "Recovered malformed request 1");
    expect((await actionCardsOf(transport))[0]).toMatchObject({
      status: "unsupported",
      params: { variant: "unknown" },
    });

    await collectTurn(transport, "Recovered malformed request 2");
    expect((await actionCardsOf(transport))[0]).toMatchObject({
      status: "unsupported",
      params: { variant: "unknown" },
    });

    await collectTurn(transport, "Recovered malformed request 3");
    expect((await actionCardsOf(transport))[0]).toMatchObject({
      status: "conflicted",
      outcome_note:
        "This action request was reissued with conflicting details. NyxID kept the first request and disabled this card.",
    });
  });

  it("keeps a blocked-to-unsupported downgrade one-way across later serviceable reissues", async () => {
    perTurnStub([
      actionRequestFrame(),
      actionRequestFrame({ schemaVersion: 5 }),
      actionRequestFrame(),
    ]);
    const transport = new AevatarAssistantTransport();
    await seedActorConversation(transport);

    await collectTurn(transport, "Downgrade step 1");
    const [card] = await actionCardsOf(transport);
    if (!card) throw new Error("no card");
    transport.blockActionCard(CONVERSATION_ID, card.block_id, "no id");

    await collectTurn(transport, "Downgrade step 2");
    expect((await actionCardsOf(transport))[0]).toMatchObject({
      status: "unsupported",
      outcome_note: "no id",
    });

    await collectTurn(transport, "Downgrade step 3");
    expect((await actionCardsOf(transport))[0]?.status).toBe("unsupported");
  });

  it("renders fail-closed unsupported cards for invalid or secret-shaped params", async () => {
    stubFetch(
      routeStream([
        { type: "RUN_STARTED", turnId: TURN_ID },
        actionRequestFrame({
          actionRequestId: "act-http-url",
          params: {
            customService: {
              name: "Build API",
              endpointUrl: "http://build.example.test/v1",
            },
          },
        }),
        actionRequestFrame({
          actionRequestId: "act-query-url",
          params: {
            customService: {
              name: "Build API",
              endpointUrl: "https://build.example.test/v1?token=nope",
            },
          },
        }),
        actionRequestFrame({
          actionRequestId: "act-fragment-url",
          params: {
            customService: {
              name: "Build API",
              endpointUrl: "https://build.example.test/v1#secret",
            },
          },
        }),
        actionRequestFrame({
          actionRequestId: "act-bad-slug",
          params: { catalogService: { serviceSlug: "api github" } },
        }),
        actionRequestFrame({
          actionRequestId: "act-secret-value",
          params: {
            customService: {
              name: "Build API",
              endpointUrl: "https://build.example.test/v1",
              targetOrgId: "Bearer top-secret-value",
            },
          },
        }),
        actionRequestFrame({
          actionRequestId: "act-scope-overflow",
          params: {
            catalogService: {
              serviceSlug: "api-github",
              requestedScopes: Array.from({ length: 65 }, (_, index) => {
                return `scope-${String(index)}`;
              }),
            },
          },
        }),
        { type: "RUN_FINISHED", runFinished: { status: "blocked" } },
      ]),
    );
    const transport = new AevatarAssistantTransport();
    await seedActorConversation(transport);
    await collectTurn(transport, "Connect the requested services");

    const history = await transport.getHistory(CONVERSATION_ID);
    const cards = history.messages
      .flatMap((message) => message.blocks)
      .filter((block) => block.type === "action_card");
    expect(cards).toHaveLength(6);
    expect(cards.map((card) => card.action_request_id)).toEqual([
      "act-http-url",
      "act-query-url",
      "act-fragment-url",
      "act-bad-slug",
      "act-secret-value",
      "act-scope-overflow",
    ]);
    for (const card of cards) {
      expect(card).toMatchObject({
        status: "unsupported",
        params: { variant: "unknown" },
      });
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
    await seedActorConversation(transport);

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

describe("assistant prose fences stay inert", () => {
  const FENCE = [
    "```nyxid:connect",
    JSON.stringify({ catalog_slug: "api-github", reason: "read merged PRs" }),
    "```",
  ].join("\n");
  const CONTENT = `I need GitHub first.\n${FENCE}\nThen I'll summarise.`;

  it("renders assistant history fences as plain text", async () => {
    stubFetch(
      routeHistory([
        {
          id: "m2",
          role: "assistant",
          content: CONTENT,
          timestamp: 1784192899074,
        },
      ]),
    );
    const transport = new AevatarAssistantTransport();

    const history = await transport.getHistory(CONVERSATION_ID);

    expect(history.messages[0]?.blocks).toEqual([
      { type: "text", block_id: "m2-text", text: CONTENT },
    ]);
  });

  it("renders streamed assistant fences as plain text and never creates a connect card", async () => {
    stubFetch(
      routeStream([
        { type: "RUN_STARTED", turnId: TURN_ID, actorId: CONVERSATION_ID },
        { type: "TEXT_MESSAGE_START", textMessageStart: { messageId: "m9" } },
        {
          type: "TEXT_MESSAGE_CONTENT",
          textMessageContent: { delta: CONTENT },
        },
        { type: "TEXT_MESSAGE_END" },
        { type: "RUN_FINISHED" },
      ]),
      routeHistory([]),
    );
    const transport = new AevatarAssistantTransport();
    await seedActorConversation(transport);

    const events = await collectTurn(transport, "Summarise my merged PRs");

    expect(
      events.some(
        (event) =>
          event.event === "block.started" &&
          event.block.type === "connect_card",
      ),
    ).toBe(false);
    expect(
      events
        .filter(
          (event): event is Extract<TurnEvent, { event: "block.delta" }> =>
            event.event === "block.delta",
        )
        .map((event) => event.text)
        .join(""),
    ).toBe(CONTENT);
    const completedText = events.find(
      (event): event is Extract<TurnEvent, { event: "block.completed" }> =>
        event.event === "block.completed" && event.block_id === "m9-text",
    );
    expect(completedText?.block).toEqual({
      type: "text",
      block_id: "m9-text",
      text: CONTENT,
    });
  });

  it("renders wrapped history responses with fences as plain text", async () => {
    stubFetch((url, init) =>
      url.startsWith(`${ASSISTANT_BASE}/conversations/`) &&
      (init?.method ?? "GET") === "GET"
        ? jsonResponse({
            messages: [
              {
                id: "w1",
                role: "assistant",
                content: CONTENT,
                timestamp: 1784192899074,
              },
            ],
            stateVersion: 42,
          })
        : undefined,
    );
    const transport = new AevatarAssistantTransport();

    const history = await transport.getHistory(CONVERSATION_ID);

    expect(
      history.messages.find((message) => message.id === "w1")?.blocks,
    ).toEqual([{ type: "text", block_id: "w1-text", text: CONTENT }]);
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
    stubFetch();
    const transport = new AevatarAssistantTransport();
    const conversation = await seedActorConversation(transport);

    const history = await transport.getHistory(conversation.id);

    expect(history.messages).toEqual([]);
    expect(history.conversation.id).toBe(CONVERSATION_ID);
  });

  it.each([
    [
      "an Aevatar error envelope",
      () =>
        jsonResponse(
          { code: "CONVERSATION_NOT_FOUND", message: "Missing." },
          404,
        ),
    ],
    ["an empty body", () => new Response(null, { status: 404 })],
  ])("types a 404 with %s as not-found", async (_label, response) => {
    stubFetch((url, init) =>
      url.endsWith("/nyxid-chat-never-created") &&
      (init?.method ?? "GET") === "GET"
        ? response()
        : undefined,
    );
    const transport = new AevatarAssistantTransport();

    await expect(
      transport.getHistory("nyxid-chat-never-created"),
    ).rejects.toBeInstanceOf(AssistantConversationNotFoundError);
  });

  it("types an unrecoverable pending placeholder as not-found without a request", async () => {
    const mock = stubFetch();
    const transport = new AevatarAssistantTransport();

    await expect(
      transport.getHistory("nyxid-pending-lost-after-reload"),
    ).rejects.toBeInstanceOf(AssistantConversationNotFoundError);
    expect(mock).not.toHaveBeenCalled();
  });

  it("still rejects a transient failure on a conversation with no local transcript", async () => {
    // 5xx is a real read failure, not the not-yet-materialized state — it
    // must not be dressed up as a legitimately empty chat.
    stubFetch((url, init) =>
      url.startsWith(`${ASSISTANT_BASE}/conversations/`) &&
      (init?.method ?? "GET") === "GET"
        ? jsonResponse(
            { error: "internal", error_code: -1, message: "boom" },
            500,
          )
        : undefined,
    );
    const transport = new AevatarAssistantTransport();
    const conversation = await seedActorConversation(transport);

    await expect(transport.getHistory(conversation.id)).rejects.toThrow();
  });
});

describe("studio new chats and typed actor compatibility", () => {
  const TYPED_URL = "/api/v1/assistant/chat";
  const TYPED_TURN = "turn-typed-create-1";
  const WORKFLOW_URL = "/api/v1/assistant/workflow-chat";
  const WORKFLOW_CONVERSATION = "chatc-8bd999c402fb37d60cdcd81e3b78cfd";
  const WORKFLOW_TURN = "turn-d619940adcd817c4aeb5d1c3e57f1ca5";
  const RUN_ACTOR = "workflow-definition:studio:run:43bfe86961b44fc2a6422d0b";
  const ACTION_ACTOR = "nyxid-chat-workflow-action-1";

  function workflowContextFrame(stateVersion: string): ChatStreamFrame {
    return {
      timestamp: "1785297207163",
      custom: {
        name: "aevatar.chat.context",
        payload: {
          "@type":
            "type.googleapis.com/aevatar.workflow.runs.WorkflowChatContextPayload",
          scopeId: USER_ID,
          conversationId: WORKFLOW_CONVERSATION,
          turnId: WORKFLOW_TURN,
          stateVersion,
        },
      },
    };
  }

  const WORKFLOW_PREAMBLE: ChatStreamFrame[] = [
    workflowContextFrame("3"),
    {
      custom: {
        name: "aevatar.run.context",
        payload: {
          "@type":
            "type.googleapis.com/aevatar.workflow.runs.WorkflowRunContextPayload",
          actorId: RUN_ACTOR,
          workflowName: "studio",
          commandId: "00e6f0aa-8670-4405-9911-7903a6616cbd",
        },
      },
    },
    { runStarted: { threadId: RUN_ACTOR, runId: RUN_ACTOR } },
    { stepStarted: { stepName: "reply" } },
  ];

  const WORKFLOW_TAIL = [
    { stepFinished: { stepName: "reply" } },
    { usage: {} },
    {
      runFinished: {
        threadId: RUN_ACTOR,
        result: {
          "@type":
            "type.googleapis.com/aevatar.workflow.runs.WorkflowRunResultPayload",
          output: "Here's what's available on your NyxID account.",
        },
      },
    },
    { stateSnapshot: { snapshot: { actorId: RUN_ACTOR } } },
  ];

  function routeWorkflow(frames: unknown[]): FetchRoute {
    return (url, init) =>
      url === WORKFLOW_URL && init?.method === "POST"
        ? sseResponse(frames)
        : undefined;
  }

  async function seedWorkflowConversation(
    transport: AevatarAssistantTransport,
  ): Promise<Conversation> {
    const active = globalThis.fetch;
    vi.stubGlobal(
      "fetch",
      vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        if (
          url === `${ASSISTANT_BASE}/conversations` &&
          (init?.method ?? "GET") === "GET"
        ) {
          return Promise.resolve(
            jsonResponse({
              conversations: [
                {
                  id: WORKFLOW_CONVERSATION,
                  title: "Legacy workflow conversation",
                  updatedAt: "2026-07-29T00:00:00.000Z",
                },
              ],
            }),
          );
        }
        if (
          url === `${ASSISTANT_BASE}/conversations/${WORKFLOW_CONVERSATION}` &&
          (init?.method ?? "GET") === "GET"
        ) {
          return Promise.resolve(
            jsonResponse({
              messages: [
                {
                  id: "workflow-seed-assistant",
                  role: "assistant",
                  content: "Earlier reply",
                  timestamp: 1785297100000,
                  turnId: "turn-workflow-seed",
                },
              ],
              stateVersion: 2,
            }),
          );
        }
        return active(input, init);
      }),
    );
    try {
      (transport as unknown as { listFetchedAt: number }).listFetchedAt = 0;
      const conversations = await transport.listConversations();
      const seeded = conversations.find(
        (conversation) => conversation.id === WORKFLOW_CONVERSATION,
      );
      if (!seeded) throw new Error("workflow conversation did not merge");
      await transport.getHistory(WORKFLOW_CONVERSATION);
      return seeded;
    } finally {
      vi.stubGlobal("fetch", active);
      (transport as unknown as { listFetchedAt: number }).listFetchedAt = 0;
    }
  }

  function collectWorkflowTurn(
    transport: AevatarAssistantTransport,
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

  function workflowHistory(
    stateVersion: number,
    turnId = "turn-workflow-seed",
  ): {
    readonly messages: readonly Record<string, unknown>[];
    readonly stateVersion: number;
  } {
    return {
      messages: [
        {
          id: `assistant-${turnId}`,
          role: "assistant",
          content: "Persisted reply",
          timestamp: 1785297100000,
          turnId,
        },
      ],
      stateVersion,
    };
  }

  function workflowInternals(transport: AevatarAssistantTransport) {
    return transport as unknown as {
      activeConversationId: string | null;
      conversations: Map<
        string,
        {
          turnState: TurnReducerState;
          requiredTurnId?: string | null;
          stateVersion?: number;
          lastLocalTurnCompletedAt?: number;
        }
      >;
      applyHistoryResponse(
        conversationId: string,
        body: unknown,
      ): { turnState: TurnReducerState };
    };
  }

  function reservationUnavailable(): ChatStreamHeadersResult {
    return {
      kind: "http_error",
      status: 503,
      body: JSON.stringify({
        code: "CHAT_HISTORY_RESERVATION_UNAVAILABLE",
        message: "Conversation history is still materializing.",
      }),
    };
  }

  it("cancels a studio first turn before its placeholder is canonicalized", async () => {
    const mock = stubFetch(routeWorkflow(WORKFLOW_PREAMBLE));
    const transport = new AevatarAssistantTransport();
    const conversation = await transport.createConversation();
    const events: TurnEvent[] = [];

    transport.sendMessage(conversation.id, "cancel before start", (event) => {
      events.push(event);
    });
    transport.cancelActiveTurn(conversation.id);
    await vi.waitFor(() => {
      expect(events.at(-1)).toMatchObject({
        event: "turn.completed",
        status: "cancelled",
      });
    });

    expect(
      mock.mock.calls.some(([input]) => String(input) === WORKFLOW_URL),
    ).toBe(false);
    expect((await transport.getHistory(conversation.id)).conversation.id).toBe(
      conversation.id,
    );
  });

  it("resolves a canonical cancel address back to its placeholder-keyed run", async () => {
    const encoder = new TextEncoder();
    let preambleSent = false;
    let releasePendingPull: (() => void) | undefined;
    const mock = stubFetch((url, init) => {
      if (url === WORKFLOW_URL && init?.method === "POST") {
        return new Response(
          new ReadableStream<Uint8Array>({
            pull(controller) {
              if (!preambleSent) {
                preambleSent = true;
                controller.enqueue(
                  encoder.encode(
                    WORKFLOW_PREAMBLE.map(
                      (frame) => `data: ${JSON.stringify(frame)}\n\n`,
                    ).join(""),
                  ),
                );
                return;
              }
              return new Promise<void>((resolve) => {
                releasePendingPull = resolve;
              });
            },
            cancel: () => releasePendingPull?.(),
          }),
          { status: 200, headers: { "Content-Type": "text/event-stream" } },
        );
      }
      return undefined;
    });
    const transport = new AevatarAssistantTransport();
    const conversation = await transport.createConversation();
    const events: TurnEvent[] = [];

    await new Promise<void>((resolve) => {
      transport.sendMessage(
        conversation.id,
        "cancel by canonical id",
        (event) => {
          events.push(event);
          if (event.event === "turn.status" && event.status === "running") {
            // `aevatar.chat.context` has already aliased the placeholder to
            // the server `chatc-…` id; cancelling by that address must find
            // the run still keyed under the placeholder.
            transport.cancelActiveTurn(WORKFLOW_CONVERSATION);
          }
          if (event.event === "turn.completed") resolve();
        },
      );
    });

    expect(events.at(-1)).toMatchObject({
      event: "turn.completed",
      status: "cancelled",
    });
    expect((await transport.getHistory(conversation.id)).conversation.id).toBe(
      WORKFLOW_CONVERSATION,
    );
    // The workflow surface serves no `:stop`; the cancel is client-side only.
    expect(
      mock.mock.calls.some(([input, init]) =>
        isTypedCommandRequest(
          String(input),
          init as RequestInit | undefined,
          "task.stop",
        ),
      ),
    ).toBe(false);
  });

  it("creates a studio conversation on the first turn and preserves its blocked action contract", async () => {
    const actionBodies: Record<string, unknown>[] = [];
    const mock = stubFetch(
      routeWorkflow([
        ...WORKFLOW_PREAMBLE,
        actionRequestFrame({
          actorId: ACTION_ACTOR,
          originTurnId: WORKFLOW_TURN,
          params: {
            catalogService: {
              serviceSlug: "api-aws-cost-explorer",
              requestedScopes: ["billing:read"],
            },
          },
        }),
        { runFinished: { threadId: RUN_ACTOR, status: "blocked" } },
      ]),
      (url, init) => {
        if (
          url !== `${ASSISTANT_BASE}/conversations/${ACTION_ACTOR}/stream` ||
          init?.method !== "POST"
        ) {
          return undefined;
        }
        actionBodies.push(
          JSON.parse(String(init.body)) as Record<string, unknown>,
        );
        return sseResponse([
          {
            type: "RUN_STARTED",
            actorId: ACTION_ACTOR,
            turnId: "turn-action-continue-1",
          },
          { type: "RUN_FINISHED" },
        ]);
      },
    );
    const transport = new AevatarAssistantTransport();
    const conversation = await transport.createConversation();
    expect(conversation.id.startsWith("workflow-pending-")).toBe(true);
    expect(conversation.id.startsWith("nyxid-pending-")).toBe(false);
    expect(mock).not.toHaveBeenCalled();

    const events = await collectWorkflowTurn(
      transport,
      conversation.id,
      "show api-aws-cost-explorer billing",
    );

    const turnCall = mock.mock.calls.find(
      ([input]) => String(input) === WORKFLOW_URL,
    );
    expect(turnCall).toBeDefined();
    const body = JSON.parse(String(turnCall?.[1]?.body)) as {
      prompt: string;
      commandId: string;
      sessionId: string;
    };
    // A new conversation sends prompt + commandId + sessionId ONLY: no
    // `conversationId` (the backend fills `conversation.conversationId:
    // null`) and no `type`, whose presence would divert the request to the
    // typed actor handler instead of the studio workflow engine.
    expect(Object.keys(body)).toEqual(["prompt", "commandId", "sessionId"]);
    expect(body).toEqual({
      prompt: "show api-aws-cost-explorer billing",
      commandId: body.commandId,
      sessionId: body.sessionId,
    });
    expect(body.commandId).toMatch(/^[0-9a-f-]{36}$/);
    expect(body.sessionId).toMatch(/^[0-9a-f-]{36}$/);
    expect(
      mock.mock.calls.some(
        ([input, init]) =>
          String(input) === TYPED_URL &&
          (JSON.parse(String(init?.body)) as { type?: string }).type === "text",
      ),
    ).toBe(false);

    const terminal = events[events.length - 1];
    expect(terminal?.event === "turn.completed" && terminal.status).toBe(
      "blocked",
    );
    const actionCards = events.filter(
      (event) =>
        event.event === "block.started" && event.block.type === "action_card",
    );
    expect(actionCards).toHaveLength(1);
    expect(actionCards[0]).toMatchObject({
      block: {
        action: "service.connect",
        origin_turn_id: WORKFLOW_TURN,
        params: {
          service_slug: "api-aws-cost-explorer",
        },
      },
    });

    // The first `aevatar.chat.context` frame aliases the placeholder to the
    // server-minted chat-history id.
    const history = await transport.getHistory(conversation.id);
    expect(history.conversation.id).toBe(WORKFLOW_CONVERSATION);
    expect(history.messages.length).toBeGreaterThanOrEqual(2);
    (transport as unknown as { listFetchedAt: number }).listFetchedAt =
      Date.now();
    const listed = await transport.listConversations();
    expect(
      listed.filter((item) => item.id === WORKFLOW_CONVERSATION),
    ).toHaveLength(1);
    expect(listed.some((item) => item.id === conversation.id)).toBe(false);

    await new Promise<void>((resolve) => {
      const handle = transport.continueActions(
        conversation.id,
        WORKFLOW_TURN,
        [
          {
            actionRequestId: "act-action-1",
            originTurnId: WORKFLOW_TURN,
            disposition: "declined",
          },
        ],
        (event) => {
          if (event.event === "turn.completed") resolve();
        },
      );
      expect(handle).not.toBeNull();
    });
    // `action.continue` stays on the actor protocol, addressed to the actor
    // the card frame named — never the `chatc-…` id, never `/workflow-chat`.
    expect(actionBodies).toHaveLength(1);
    expect(actionBodies[0]).toMatchObject({
      type: "action.continue",
      conversationId: ACTION_ACTOR,
      originTurnId: WORKFLOW_TURN,
      actions: [
        {
          actionRequestId: "act-action-1",
          originTurnId: WORKFLOW_TURN,
          disposition: "declined",
        },
      ],
    });
  });

  it("keeps action-request fingerprints after studio adoption and still conflicts later reissues", async () => {
    const FOLLOW_UP_TURN = "turn-workflow-follow-up-2";
    const streamMock = mockChatStreams((request) => {
      if (request.url !== WORKFLOW_URL) return undefined;
      const body = JSON.parse(request.bodyText) as { conversationId?: string };
      if (!body.conversationId) {
        return {
          frames: [
            ...WORKFLOW_PREAMBLE,
            actionRequestFrame({
              actorId: ACTION_ACTOR,
              originTurnId: WORKFLOW_TURN,
            }),
            { runFinished: { threadId: RUN_ACTOR, status: "blocked" } },
          ],
        };
      }
      if (body.conversationId === WORKFLOW_CONVERSATION) {
        return {
          frames: [
            {
              timestamp: "1785297207164",
              custom: {
                name: "aevatar.chat.context",
                payload: {
                  "@type":
                    "type.googleapis.com/aevatar.workflow.runs.WorkflowChatContextPayload",
                  scopeId: USER_ID,
                  conversationId: WORKFLOW_CONVERSATION,
                  turnId: FOLLOW_UP_TURN,
                  stateVersion: "4",
                },
              },
            },
            { runStarted: { threadId: RUN_ACTOR, runId: RUN_ACTOR } },
            actionRequestFrame({
              actorId: ACTION_ACTOR,
              originTurnId: FOLLOW_UP_TURN,
              params: {
                catalogService: {
                  serviceSlug: "api-lark",
                  requestedScopes: ["messages:write"],
                },
              },
            }),
            { runFinished: { threadId: RUN_ACTOR, status: "blocked" } },
          ],
        };
      }
      return undefined;
    });
    const transport = new AevatarAssistantTransport();
    const conversation = await transport.createConversation();

    await collectWorkflowTurn(transport, conversation.id, "Connect GitHub");
    await collectWorkflowTurn(
      transport,
      conversation.id,
      "Connect something else",
    );

    expect(streamMock.mock.calls.map(([request]) => request.url)).toEqual([
      WORKFLOW_URL,
      WORKFLOW_URL,
    ]);

    const history = await transport.getHistory(conversation.id);
    expect(history.conversation.id).toBe(WORKFLOW_CONVERSATION);
    const cards = history.messages
      .flatMap((message) => message.blocks)
      .filter(
        (block): block is ActionCardContentBlock =>
          block.type === "action_card",
      );
    expect(cards).toHaveLength(1);
    expect(cards[0]).toMatchObject({
      action_request_id: "act-action-1",
      origin_turn_id: WORKFLOW_TURN,
      status: "conflicted",
      outcome_note:
        "This action request was reissued with conflicting details. NyxID kept the first request and disabled this card.",
      params: {
        variant: "catalog",
        service_slug: "api-github",
        requested_scopes: ["repo"],
      },
    });
  });

  it("continues existing typed conversations on their actor with a fresh request identity", async () => {
    let turnCounter = 0;
    const streamMock = mockChatStreams((request) => {
      const body = JSON.parse(request.bodyText) as {
        type: string;
        conversationId?: string;
      };
      if (
        request.url === TYPED_URL &&
        body.type === "text" &&
        body.conversationId === CONVERSATION_ID
      ) {
        turnCounter += 1;
        return {
          frames: [
            {
              type: "RUN_STARTED",
              actorId: CONVERSATION_ID,
              turnId: `turn-typed-follow-up-${turnCounter}`,
            },
            { type: "RUN_FINISHED" },
          ],
        };
      }
      return undefined;
    });
    const transport = new AevatarAssistantTransport();
    // A `nyxid-chat-…` conversation predates the studio cutover; its
    // transcript stays continuable on the typed actor surface.
    const conversation = await seedActorConversation(transport);

    await collectWorkflowTurn(transport, conversation.id, "first");
    await collectWorkflowTurn(transport, conversation.id, "second");

    const bodies = streamMock.mock.calls.map(([request]) => ({
      url: request.url,
      body: JSON.parse(request.bodyText) as Record<string, unknown>,
    }));
    expect(bodies.map((entry) => entry.url)).toEqual([TYPED_URL, TYPED_URL]);
    expect(bodies[0]?.body).toMatchObject({
      type: "text",
      conversationId: CONVERSATION_ID,
      prompt: "first",
    });
    expect(bodies[1]?.body).toMatchObject({
      type: "text",
      conversationId: CONVERSATION_ID,
      prompt: "second",
    });
    expect(bodies[0]?.body["clientRequestId"]).not.toBe(
      bodies[1]?.body["clientRequestId"],
    );
    expect(
      streamMock.mock.calls.some(([request]) => request.url === WORKFLOW_URL),
    ).toBe(false);
  });

  it("wakes an existing typed conversation out of band with an empty action list", async () => {
    const streamMock = mockChatStreams((request) => {
      const body = JSON.parse(request.bodyText) as {
        type: string;
        conversationId?: string;
      };
      if (request.url !== TYPED_URL) return undefined;
      if (body.type === "text" && body.conversationId === CONVERSATION_ID) {
        return {
          frames: [
            {
              type: "RUN_STARTED",
              actorId: CONVERSATION_ID,
              turnId: TYPED_TURN,
            },
            {
              type: "RUN_FINISHED",
              runFinished: {
                status: "blocked",
              },
            },
          ],
        };
      }
      if (
        body.type === "action.continue" &&
        body.conversationId === CONVERSATION_ID
      ) {
        return {
          frames: [
            {
              type: "RUN_STARTED",
              actorId: CONVERSATION_ID,
              turnId: "turn-typed-wake-1",
            },
            {
              type: "RUN_FINISHED",
              runFinished: {
                status: "completed",
              },
            },
          ],
        };
      }
      return undefined;
    });
    const transport = new AevatarAssistantTransport();
    const conversation = await seedActorConversation(transport);
    await collectWorkflowTurn(transport, conversation.id, "wait for access");

    await new Promise<void>((resolve) => {
      transport.wakeActions(conversation.id, TYPED_TURN, (event) => {
        if (event.event === "turn.completed") resolve();
      });
    });

    const bodies = streamMock.mock.calls.map(([request]) => ({
      url: request.url,
      body: JSON.parse(request.bodyText) as Record<string, unknown>,
    }));
    expect(bodies.map((entry) => entry.url)).toEqual([TYPED_URL, TYPED_URL]);
    expect(Object.keys(bodies[1]?.body ?? {})).toEqual([
      "type",
      "conversationId",
      "clientRequestId",
      "originTurnId",
      "actions",
    ]);
    expect(bodies[1]?.body).toEqual({
      type: "action.continue",
      conversationId: CONVERSATION_ID,
      clientRequestId: bodies[1]?.body["clientRequestId"],
      originTurnId: TYPED_TURN,
      actions: [],
    });
    expect(bodies[1]?.body["clientRequestId"]).not.toBe(
      bodies[0]?.body["clientRequestId"],
    );
    expect(
      streamMock.mock.calls.some(([request]) => request.url === WORKFLOW_URL),
    ).toBe(false);
  });

  it("refuses to wake a legacy workflow conversation", async () => {
    const mock = stubFetch();
    const transport = new AevatarAssistantTransport();
    const conversation = await seedWorkflowConversation(transport);

    expect(() => transport.wakeActions(conversation.id, WORKFLOW_TURN)).toThrow(
      /typed assistant conversation/,
    );
    expect(
      mock.mock.calls.some(
        ([input, init]) =>
          String(input) === WORKFLOW_URL && init?.method === "POST",
      ),
    ).toBe(false);
  });

  it("fails closed when a studio stream does not identify its turn", async () => {
    stubFetch(
      routeWorkflow([
        {
          timestamp: "1785297207163",
          custom: {
            name: "aevatar.chat.context",
            payload: {
              "@type":
                "type.googleapis.com/aevatar.workflow.runs.WorkflowChatContextPayload",
              scopeId: USER_ID,
              conversationId: WORKFLOW_CONVERSATION,
              // No turnId: nothing downstream can be attributed to a turn.
              stateVersion: "3",
            },
          },
        },
        { runStarted: { threadId: RUN_ACTOR, runId: RUN_ACTOR } },
        { runFinished: { threadId: RUN_ACTOR } },
      ]),
    );
    const transport = new AevatarAssistantTransport();
    const conversation = await transport.createConversation();

    const events = await collectWorkflowTurn(transport, conversation.id, "hi");

    expect(events.at(-1)).toMatchObject({
      event: "turn.completed",
      status: "failed",
      error: { code: "stream_protocol_error" },
    });
  });

  it("keeps legacy chatc history continuable and renders its workflow result", async () => {
    stubFetch(routeWorkflow([...WORKFLOW_PREAMBLE, ...WORKFLOW_TAIL]));
    const transport = new AevatarAssistantTransport();
    const conversation = await seedWorkflowConversation(transport);

    const events = await collectWorkflowTurn(transport, conversation.id, "hi");

    const textBlocks = events.filter(
      (event) =>
        event.event === "block.completed" && event.block.type === "text",
    );
    expect(
      textBlocks.some(
        (event) =>
          event.event === "block.completed" &&
          event.block.type === "text" &&
          event.block.text === "Here's what's available on your NyxID account.",
      ),
    ).toBe(true);
    expect(events[events.length - 1]?.event).toBe("turn.completed");
  });

  it("continues with the observed stateVersion and no client commandId", async () => {
    const mock = stubFetch(
      routeWorkflow([
        ...WORKFLOW_PREAMBLE,
        { textMessageStart: { messageId: "wm-1" } },
        { textMessageContent: { delta: "First." } },
        { textMessageEnd: {} },
        ...WORKFLOW_TAIL,
      ]),
    );
    const transport = new AevatarAssistantTransport();
    const conversation = await seedWorkflowConversation(transport);
    await collectWorkflowTurn(transport, conversation.id, "first");
    await collectWorkflowTurn(transport, conversation.id, "second");

    const turnBodies = mock.mock.calls
      .filter(([input]) => String(input) === WORKFLOW_URL)
      .map(
        ([, init]) => JSON.parse(String(init?.body)) as Record<string, unknown>,
      );
    expect(turnBodies).toHaveLength(2);
    // Existing chatc rows stay on the workflow route and use the transcript
    // watermark hydrated by the seed helper. No watermark is fabricated.
    expect(turnBodies[0]?.["conversationId"]).toBe(WORKFLOW_CONVERSATION);
    expect(turnBodies[0]?.["minimumStateVersion"]).toBe(2);
    expect(Object.keys(turnBodies[0] ?? {})).toEqual([
      "prompt",
      "conversationId",
      "minimumStateVersion",
      "sessionId",
    ]);
    // The follow-up uses the watermark from the first turn's chat.context.
    expect(turnBodies[1]?.["conversationId"]).toBe(WORKFLOW_CONVERSATION);
    expect(turnBodies[1]?.["minimumStateVersion"]).toBe(3);
    expect(turnBodies[0]).not.toHaveProperty("commandId");
    expect(turnBodies[1]).not.toHaveProperty("commandId");
  });

  it("reuses one sessionId for every turn of a conversation", async () => {
    const mock = stubFetch(
      routeWorkflow([
        ...WORKFLOW_PREAMBLE,
        { textMessageStart: { messageId: "wm-1" } },
        { textMessageContent: { delta: "First." } },
        { textMessageEnd: {} },
        ...WORKFLOW_TAIL,
      ]),
    );
    const transport = new AevatarAssistantTransport();
    // Starts on the client placeholder, adopts `chatc-…` from the first
    // turn's chat.context: the session must survive that re-key.
    const conversation = await transport.createConversation();
    await collectWorkflowTurn(transport, conversation.id, "first");
    await transport.getHistory(conversation.id);
    await collectWorkflowTurn(transport, conversation.id, "second");

    const bodies = mock.mock.calls
      .filter(([input]) => String(input) === WORKFLOW_URL)
      .map(
        ([, init]) => JSON.parse(String(init?.body)) as Record<string, unknown>,
      );
    expect(bodies).toHaveLength(2);
    const sessionId = bodies[0]?.["sessionId"];
    expect(sessionId).toMatch(/^[0-9a-f-]{36}$/);
    // A same-conversation history re-read must not count as a reopen or remint.
    expect(bodies[1]?.["sessionId"]).toBe(sessionId);
    expect(bodies[1]).not.toHaveProperty("commandId");
    // The create turn carries no conversationId, so it stays on the studio
    // branch of the upstream dispatcher.
    expect(bodies[0]?.["conversationId"]).toBeUndefined();
    expect(bodies[1]?.["conversationId"]).toBe(WORKFLOW_CONVERSATION);
  });

  it("refreshes and raises the fence only for the named reservation 503", async () => {
    vi.useFakeTimers();
    try {
      let streamAttempt = 0;
      const streams = mockChatStreams((request) => {
        if (request.url !== WORKFLOW_URL) return undefined;
        streamAttempt += 1;
        if (streamAttempt === 1) return { headers: reservationUnavailable() };
        return {
          frames: [
            workflowContextFrame("5"),
            { runStarted: { threadId: RUN_ACTOR, runId: RUN_ACTOR } },
            ...WORKFLOW_TAIL,
          ],
        };
      });
      stubFetch((url, init) =>
        url === `${ASSISTANT_BASE}/conversations/${WORKFLOW_CONVERSATION}` &&
        (init?.method ?? "GET") === "GET"
          ? jsonResponse(workflowHistory(5))
          : undefined,
      );
      const transport = new AevatarAssistantTransport();
      const conversation = await seedWorkflowConversation(transport);

      const turn = collectWorkflowTurn(transport, conversation.id, "continue");
      await vi.advanceTimersByTimeAsync(300);
      const events = await turn;

      expect(events.at(-1)).toMatchObject({
        event: "turn.completed",
        status: "completed",
      });
      const bodies = streams.mock.calls.map(
        ([request]) => JSON.parse(request.bodyText) as Record<string, unknown>,
      );
      expect(bodies).toHaveLength(2);
      expect(bodies[0]?.["minimumStateVersion"]).toBe(2);
      expect(bodies[1]?.["minimumStateVersion"]).toBe(5);
      expect(bodies[1]?.["sessionId"]).toBe(bodies[0]?.["sessionId"]);
      expect(bodies[0]).not.toHaveProperty("commandId");
      expect(bodies[1]).not.toHaveProperty("commandId");
    } finally {
      vi.useRealTimers();
    }
  });

  it("waits for a below-fence refresh before retrying the continuation", async () => {
    vi.useFakeTimers();
    try {
      let streamAttempt = 0;
      let historyAttempt = 0;
      const streams = mockChatStreams((request) => {
        if (request.url !== WORKFLOW_URL) return undefined;
        streamAttempt += 1;
        return streamAttempt === 1
          ? { headers: reservationUnavailable() }
          : {
              frames: [
                workflowContextFrame("4"),
                { runStarted: { threadId: RUN_ACTOR, runId: RUN_ACTOR } },
                ...WORKFLOW_TAIL,
              ],
            };
      });
      stubFetch((url, init) => {
        if (
          url !== `${ASSISTANT_BASE}/conversations/${WORKFLOW_CONVERSATION}` ||
          (init?.method ?? "GET") !== "GET"
        ) {
          return undefined;
        }
        historyAttempt += 1;
        return jsonResponse(workflowHistory(historyAttempt === 1 ? 1 : 4));
      });
      const transport = new AevatarAssistantTransport();
      const conversation = await seedWorkflowConversation(transport);

      const turn = collectWorkflowTurn(transport, conversation.id, "continue");
      await vi.advanceTimersByTimeAsync(1_200);
      await turn;

      expect(historyAttempt).toBe(2);
      expect(streams).toHaveBeenCalledTimes(2);
      expect(JSON.parse(streams.mock.calls[1]![0].bodyText)).toMatchObject({
        minimumStateVersion: 4,
      });
    } finally {
      vi.useRealTimers();
    }
  });

  it.each([
    [
      "an unrelated 503",
      {
        kind: "http_error",
        status: 503,
        body: JSON.stringify({ code: "OTHER_UNAVAILABLE", message: "No." }),
      } satisfies ChatStreamHeadersResult,
    ],
    [
      "a 500",
      {
        kind: "http_error",
        status: 500,
        body: JSON.stringify({ code: "ENGINE_FAILED", message: "Failed." }),
      } satisfies ChatStreamHeadersResult,
    ],
    [
      "an empty 503 body",
      {
        kind: "http_error",
        status: 503,
        body: "",
      } satisfies ChatStreamHeadersResult,
    ],
    [
      "a malformed 503 body",
      {
        kind: "http_error",
        status: 503,
        body: "{not-json",
      } satisfies ChatStreamHeadersResult,
    ],
    [
      "a network error",
      {
        kind: "network_error",
        code: "network_error",
        message: "offline",
      } satisfies ChatStreamHeadersResult,
    ],
  ])(
    "does not replay a workflow continuation after %s",
    async (_label, headers) => {
      const streams = mockChatStreams((request) =>
        request.url === WORKFLOW_URL ? { headers } : undefined,
      );
      stubFetch();
      const transport = new AevatarAssistantTransport();
      const conversation = await seedWorkflowConversation(transport);

      const events = await collectWorkflowTurn(
        transport,
        conversation.id,
        "continue",
      );

      expect(events.at(-1)).toMatchObject({
        event: "turn.completed",
        status: "failed",
      });
      expect(streams).toHaveBeenCalledTimes(1);
    },
  );

  it.each([404, 500])(
    "does not replay when reservation refresh remains HTTP %i",
    async (status) => {
      vi.useFakeTimers();
      try {
        const streams = mockChatStreams((request) =>
          request.url === WORKFLOW_URL
            ? { headers: reservationUnavailable() }
            : undefined,
        );
        stubFetch((url, init) =>
          url === `${ASSISTANT_BASE}/conversations/${WORKFLOW_CONVERSATION}` &&
          (init?.method ?? "GET") === "GET"
            ? jsonResponse(
                { code: "HISTORY_UNAVAILABLE", message: "Not ready." },
                status,
              )
            : undefined,
        );
        const transport = new AevatarAssistantTransport();
        const conversation = await seedWorkflowConversation(transport);

        const turn = collectWorkflowTurn(
          transport,
          conversation.id,
          "continue",
        );
        await vi.advanceTimersByTimeAsync(status === 500 ? 1_200 : 300);
        const events = await turn;

        expect(events.at(-1)).toMatchObject({
          event: "turn.completed",
          status: "failed",
          error: {
            code:
              status === 404
                ? "history_refresh_failed"
                : "CHAT_HISTORY_RESERVATION_UNAVAILABLE",
          },
        });
        expect(streams).toHaveBeenCalledTimes(1);
      } finally {
        vi.useRealTimers();
      }
    },
  );

  it("retries a status-less reservation refresh failure", async () => {
    vi.useFakeTimers();
    try {
      let streamAttempt = 0;
      let historyAttempt = 0;
      const streams = mockChatStreams((request) => {
        if (request.url !== WORKFLOW_URL) return undefined;
        streamAttempt += 1;
        return streamAttempt === 1
          ? { headers: reservationUnavailable() }
          : {
              frames: [
                workflowContextFrame("5"),
                { runStarted: { threadId: RUN_ACTOR, runId: RUN_ACTOR } },
                ...WORKFLOW_TAIL,
              ],
            };
      });
      stubFetch((url, init) => {
        if (
          url !== `${ASSISTANT_BASE}/conversations/${WORKFLOW_CONVERSATION}` ||
          (init?.method ?? "GET") !== "GET"
        ) {
          return undefined;
        }
        historyAttempt += 1;
        if (historyAttempt === 1) throw new TypeError("network disconnected");
        return jsonResponse(workflowHistory(5));
      });
      const transport = new AevatarAssistantTransport();
      const conversation = await seedWorkflowConversation(transport);

      const turn = collectWorkflowTurn(transport, conversation.id, "continue");
      await vi.advanceTimersByTimeAsync(1_200);
      const events = await turn;

      expect(events.at(-1)).toMatchObject({
        event: "turn.completed",
        status: "completed",
      });
      expect(historyAttempt).toBe(2);
      expect(streams).toHaveBeenCalledTimes(2);
    } finally {
      vi.useRealTimers();
    }
  });

  it("fails reservation refresh immediately when the conversation is tombstoned", async () => {
    vi.useFakeTimers();
    try {
      const streams = mockChatStreams((request) =>
        request.url === WORKFLOW_URL
          ? { headers: reservationUnavailable() }
          : undefined,
      );
      let historyAttempt = 0;
      const transport = new AevatarAssistantTransport();
      const deletedConversationIds = (
        transport as unknown as { deletedConversationIds: Set<string> }
      ).deletedConversationIds;
      stubFetch((url, init) => {
        if (
          url !== `${ASSISTANT_BASE}/conversations/${WORKFLOW_CONVERSATION}` ||
          (init?.method ?? "GET") !== "GET"
        ) {
          return undefined;
        }
        historyAttempt += 1;
        deletedConversationIds.add(WORKFLOW_CONVERSATION);
        return jsonResponse(workflowHistory(5));
      });
      const conversation = await seedWorkflowConversation(transport);

      const turn = collectWorkflowTurn(transport, conversation.id, "continue");
      await vi.advanceTimersByTimeAsync(300);
      const events = await turn;

      expect(events.at(-1)).toMatchObject({
        event: "turn.completed",
        status: "failed",
        error: {
          code: "history_refresh_failed",
          message: "Conversation was not found.",
        },
      });
      expect(historyAttempt).toBe(1);
      expect(streams).toHaveBeenCalledTimes(1);
    } finally {
      vi.useRealTimers();
    }
  });

  it("does not dispatch a retry when cancellation lands during the reservation wait", async () => {
    vi.useFakeTimers();
    try {
      const streams = mockChatStreams((request) =>
        request.url === WORKFLOW_URL
          ? { headers: reservationUnavailable() }
          : undefined,
      );
      stubFetch();
      const transport = new AevatarAssistantTransport();
      const conversation = await seedWorkflowConversation(transport);
      const events: TurnEvent[] = [];
      const handle = transport.sendMessage(
        conversation.id,
        "continue",
        (event) => {
          events.push(event);
        },
      );
      await Promise.resolve();
      await Promise.resolve();
      handle.cancel();
      await vi.runAllTimersAsync();

      expect(events.at(-1)).toMatchObject({
        event: "turn.completed",
        status: "cancelled",
      });
      expect(streams).toHaveBeenCalledTimes(1);
    } finally {
      vi.useRealTimers();
    }
  });

  it("preflights a missing watermark before one optimistic append", async () => {
    vi.useFakeTimers();
    try {
      let historyReady = false;
      let readyHistoryAttempt = 0;
      const streams = mockChatStreams((request) =>
        request.url === WORKFLOW_URL
          ? { frames: [...WORKFLOW_PREAMBLE, ...WORKFLOW_TAIL] }
          : undefined,
      );
      stubFetch((url, init) =>
        url === `${ASSISTANT_BASE}/conversations/${WORKFLOW_CONVERSATION}` &&
        (init?.method ?? "GET") === "GET"
          ? jsonResponse(
              historyReady
                ? ++readyHistoryAttempt === 1
                  ? { messages: [], stateVersion: 4 }
                  : workflowHistory(4)
                : { messages: [], stateVersion: 0 },
            )
          : undefined,
      );
      const transport = new AevatarAssistantTransport();
      const conversation = await seedWorkflowConversation(transport);
      const internals = transport as unknown as {
        conversations: Map<
          string,
          { stateVersion?: number; turnState: { messages: AssistantMessage[] } }
        >;
      };
      const stored = internals.conversations.get(conversation.id)!;
      stored.stateVersion = undefined;
      const originalCount = stored.turnState.messages.length;

      const failedTurn = collectWorkflowTurn(
        transport,
        conversation.id,
        "continue",
      );
      expect(stored.turnState.messages).toHaveLength(originalCount);
      await vi.advanceTimersByTimeAsync(3_000);
      const failedEvents = await failedTurn;
      expect(failedEvents.at(-1)).toMatchObject({
        event: "turn.completed",
        status: "failed",
        error: { code: "history_synchronizing" },
      });
      expect(stored.turnState.messages).toHaveLength(originalCount);
      expect(streams).not.toHaveBeenCalled();

      historyReady = true;
      const successfulTurn = collectWorkflowTurn(
        transport,
        conversation.id,
        "continue",
      );
      await vi.advanceTimersByTimeAsync(300);
      const successfulEvents = await successfulTurn;
      expect(successfulEvents.at(-1)).toMatchObject({
        event: "turn.completed",
        status: "completed",
      });
      const current = internals.conversations.get(conversation.id)!;
      expect(
        current.turnState.messages.filter((message) => message.role === "user"),
      ).toHaveLength(1);
      expect(streams).toHaveBeenCalledTimes(1);
      expect(readyHistoryAttempt).toBe(2);
      expect(JSON.parse(streams.mock.calls[0]![0].bodyText)).toMatchObject({
        minimumStateVersion: 4,
      });
    } finally {
      vi.useRealTimers();
    }
  });

  it("does not replay a continuation after an accepted stream truncates", async () => {
    const streams = mockChatStreams((request) =>
      request.url === WORKFLOW_URL
        ? {
            completion: {
              kind: "network_error",
              code: "stream_closed",
              message: "truncated",
            },
          }
        : undefined,
    );
    stubFetch();
    const transport = new AevatarAssistantTransport();
    const conversation = await seedWorkflowConversation(transport);

    const events = await collectWorkflowTurn(
      transport,
      conversation.id,
      "continue",
    );

    expect(events.at(-1)).toMatchObject({
      event: "turn.completed",
      status: "failed",
      error: { code: "stream_closed" },
    });
    expect(streams).toHaveBeenCalledTimes(1);
  });

  it("reconciles a create context at stateVersion zero before its first continuation", async () => {
    let streamAttempt = 0;
    const streams = mockChatStreams((request) => {
      if (request.url !== WORKFLOW_URL) return undefined;
      streamAttempt += 1;
      return streamAttempt === 1
        ? {
            frames: [
              workflowContextFrame("0"),
              { runStarted: { threadId: RUN_ACTOR, runId: RUN_ACTOR } },
              ...WORKFLOW_TAIL,
            ],
          }
        : {
            frames: [
              workflowContextFrame("3"),
              { runStarted: { threadId: RUN_ACTOR, runId: RUN_ACTOR } },
              ...WORKFLOW_TAIL,
            ],
          };
    });
    stubFetch((url, init) =>
      url === `${ASSISTANT_BASE}/conversations/${WORKFLOW_CONVERSATION}` &&
      (init?.method ?? "GET") === "GET"
        ? jsonResponse(workflowHistory(2, WORKFLOW_TURN))
        : undefined,
    );
    const transport = new AevatarAssistantTransport();
    const conversation = await transport.createConversation();

    await collectWorkflowTurn(transport, conversation.id, "create");
    await collectWorkflowTurn(transport, conversation.id, "continue");

    const bodies = streams.mock.calls.map(
      ([request]) => JSON.parse(request.bodyText) as Record<string, unknown>,
    );
    expect(bodies).toHaveLength(2);
    expect(bodies[0]).toHaveProperty("commandId");
    expect(bodies[1]).toEqual({
      prompt: "continue",
      conversationId: WORKFLOW_CONVERSATION,
      minimumStateVersion: 2,
      sessionId: bodies[0]?.["sessionId"],
    });
  });

  it("recovers after a context-free terminal and fails closed when recovery stays empty", async () => {
    vi.useFakeTimers();
    try {
      const streams = mockChatStreams((request) =>
        request.url === WORKFLOW_URL
          ? {
              frames: [
                { runStarted: { threadId: RUN_ACTOR, runId: RUN_ACTOR } },
                ...WORKFLOW_TAIL,
              ],
            }
          : undefined,
      );
      let recoveryAttempts = 0;
      stubFetch((url) => {
        if (!url.includes("/conversations/create-recovery/")) return undefined;
        recoveryAttempts += 1;
        return jsonResponse(
          { code: "CREATE_NOT_FOUND", message: "Not ready." },
          404,
        );
      });
      const transport = new AevatarAssistantTransport();
      const conversation = await transport.createConversation();

      const turn = collectWorkflowTurn(transport, conversation.id, "create");
      await vi.advanceTimersByTimeAsync(3_000);
      const events = await turn;

      expect(events.at(-1)).toMatchObject({
        event: "turn.completed",
        status: "failed",
        error: { code: "stream_protocol_error" },
      });
      expect(recoveryAttempts).toBe(4);
      expect(streams).toHaveBeenCalledTimes(1);
      expect(
        (await transport.getHistory(conversation.id)).conversation.id,
      ).toMatch(/^workflow-pending-/);
    } finally {
      vi.useRealTimers();
    }
  });

  it("retains a context-free error terminal when recovery stays empty", async () => {
    vi.useFakeTimers();
    try {
      mockChatStreams((request) =>
        request.url === WORKFLOW_URL
          ? {
              frames: [
                { runStarted: { threadId: RUN_ACTOR, runId: RUN_ACTOR } },
                {
                  runError: {
                    code: "WORKFLOW_FAILED",
                    message: "The upstream workflow failed permanently.",
                  },
                },
              ],
            }
          : undefined,
      );
      stubFetch((url) =>
        url.includes("/conversations/create-recovery/")
          ? jsonResponse(
              { code: "CREATE_NOT_FOUND", message: "Not ready." },
              404,
            )
          : undefined,
      );
      const transport = new AevatarAssistantTransport();
      const conversation = await transport.createConversation();

      const turn = collectWorkflowTurn(transport, conversation.id, "create");
      await vi.advanceTimersByTimeAsync(3_000);
      const events = await turn;

      expect(events.at(-1)).toMatchObject({
        event: "turn.completed",
        status: "failed",
        error: {
          code: "WORKFLOW_FAILED",
          message: "The upstream workflow failed permanently.",
        },
      });
      expect(
        (await transport.getHistory(conversation.id)).conversation.id,
      ).toMatch(/^workflow-pending-/);
    } finally {
      vi.useRealTimers();
    }
  });

  it("reads a pending placeholder from the local mirror without a doomed request", async () => {
    // A `workflow-pending-` id exists nowhere server-side, so the transcript
    // read could only 404 and fall back to the mirror it already holds. The
    // request is pure waste on every new chat.
    vi.useFakeTimers();
    try {
      mockChatStreams((request) =>
        request.url === WORKFLOW_URL
          ? {
              frames: [
                { runStarted: { threadId: RUN_ACTOR, runId: RUN_ACTOR } },
                {
                  runError: {
                    code: "WORKFLOW_FAILED",
                    message: "The upstream workflow failed permanently.",
                  },
                },
              ],
            }
          : undefined,
      );
      const mock = stubFetch((url) =>
        url.includes("/conversations/create-recovery/")
          ? jsonResponse(
              { code: "CREATE_NOT_FOUND", message: "Not ready." },
              404,
            )
          : undefined,
      );
      const transport = new AevatarAssistantTransport();
      const conversation = await transport.createConversation();
      expect(conversation.id).toMatch(/^workflow-pending-/);

      const turn = collectWorkflowTurn(
        transport,
        conversation.id,
        "keep this turn local",
      );
      await vi.advanceTimersByTimeAsync(3_000);
      await turn;

      mock.mockClear();
      const history = await transport.getHistory(conversation.id);

      const mirror = (
        transport as unknown as {
          conversations: Map<
            string,
            { turnState: { messages: readonly AssistantMessage[] } }
          >;
        }
      ).conversations.get(conversation.id);
      expect(history.conversation.id).toBe(conversation.id);
      expect(history.messages.length).toBeGreaterThan(0);
      expect(history.messages).toBe(mirror?.turnState.messages);
      expect(history.has_more).toBe(false);
      expect(history.awaitingProjection).toBe(true);
      expect(mock).not.toHaveBeenCalled();
    } finally {
      vi.useRealTimers();
    }
  });

  it("preserves an error terminal after context-free create recovery", async () => {
    const streams = mockChatStreams((request) =>
      request.url === WORKFLOW_URL
        ? {
            frames: [
              { runStarted: { threadId: RUN_ACTOR, runId: RUN_ACTOR } },
              {
                runError: {
                  code: "WORKFLOW_FAILED",
                  message: "The workflow engine rejected this turn.",
                },
              },
            ],
          }
        : undefined,
    );
    stubFetch((url, init) => {
      if (url.includes("/conversations/create-recovery/")) {
        return jsonResponse({
          status: "append_committed",
          conversationId: WORKFLOW_CONVERSATION,
          stateVersion: 3,
          turnId: WORKFLOW_TURN,
        });
      }
      if (
        url === `${ASSISTANT_BASE}/conversations/${WORKFLOW_CONVERSATION}` &&
        (init?.method ?? "GET") === "GET"
      ) {
        return jsonResponse(workflowHistory(3, WORKFLOW_TURN));
      }
      return undefined;
    });
    const transport = new AevatarAssistantTransport();
    const conversation = await transport.createConversation();

    const events = await collectWorkflowTurn(
      transport,
      conversation.id,
      "create",
    );

    expect(events.at(-1)).toMatchObject({
      event: "turn.completed",
      status: "failed",
      error: {
        code: "WORKFLOW_FAILED",
        message: "The workflow engine rejected this turn.",
      },
    });
    expect(streams).toHaveBeenCalledTimes(1);
  });

  it("preserves a blocked terminal after context-free create recovery", async () => {
    mockChatStreams((request) =>
      request.url === WORKFLOW_URL
        ? {
            frames: [
              { runStarted: { threadId: RUN_ACTOR, runId: RUN_ACTOR } },
              { runFinished: { status: "blocked" } },
            ],
          }
        : undefined,
    );
    stubFetch((url, init) => {
      if (url.includes("/conversations/create-recovery/")) {
        return jsonResponse({
          status: "append_committed",
          conversationId: WORKFLOW_CONVERSATION,
          stateVersion: 3,
          turnId: WORKFLOW_TURN,
        });
      }
      if (
        url === `${ASSISTANT_BASE}/conversations/${WORKFLOW_CONVERSATION}` &&
        (init?.method ?? "GET") === "GET"
      ) {
        return jsonResponse(workflowHistory(3, WORKFLOW_TURN));
      }
      return undefined;
    });
    const transport = new AevatarAssistantTransport();
    const conversation = await transport.createConversation();

    const events = await collectWorkflowTurn(
      transport,
      conversation.id,
      "create",
    );

    expect(events.at(-1)).toMatchObject({
      event: "turn.completed",
      status: "blocked",
    });
  });

  it("recovers a create truncated after RUN_STARTED", async () => {
    const streams = mockChatStreams((request) =>
      request.url === WORKFLOW_URL
        ? {
            frames: [{ runStarted: { threadId: RUN_ACTOR, runId: RUN_ACTOR } }],
            completion: {
              kind: "network_error",
              code: "stream_closed",
              message: "truncated",
            },
          }
        : undefined,
    );
    const fetchMock = stubFetch((url, init) => {
      if (url.includes("/conversations/create-recovery/")) {
        return jsonResponse({
          status: "append_committed",
          conversationId: WORKFLOW_CONVERSATION,
          stateVersion: 3,
          turnId: WORKFLOW_TURN,
        });
      }
      if (
        url === `${ASSISTANT_BASE}/conversations/${WORKFLOW_CONVERSATION}` &&
        (init?.method ?? "GET") === "GET"
      ) {
        return jsonResponse(workflowHistory(3, WORKFLOW_TURN));
      }
      return undefined;
    });
    const transport = new AevatarAssistantTransport();
    const conversation = await transport.createConversation();

    const events = await collectWorkflowTurn(
      transport,
      conversation.id,
      "create",
    );

    expect(events.at(-1)).toMatchObject({
      event: "turn.completed",
      status: "completed",
    });
    expect(streams).toHaveBeenCalledTimes(1);
    expect(
      fetchMock.mock.calls.some(([input]) =>
        String(input).includes("/conversations/create-recovery/"),
      ),
    ).toBe(true);
    expect((await transport.getHistory(conversation.id)).conversation.id).toBe(
      WORKFLOW_CONVERSATION,
    );
  });

  it("recovers a context-free create and waits for its assistant transcript", async () => {
    vi.useFakeTimers();
    try {
      const streams = mockChatStreams((request) =>
        request.url === WORKFLOW_URL ? {} : undefined,
      );
      let recoveryAttempt = 0;
      let historyAttempt = 0;
      const fetchMock = stubFetch((url, init) => {
        if (
          url.includes("/conversations/create-recovery/") &&
          (init?.method ?? "GET") === "GET"
        ) {
          recoveryAttempt += 1;
          return recoveryAttempt === 1
            ? jsonResponse(
                { code: "CREATE_NOT_FOUND", message: "Not ready." },
                404,
              )
            : jsonResponse({
                status: "append_committed",
                conversationId: WORKFLOW_CONVERSATION,
                stateVersion: 4,
                turnId: WORKFLOW_TURN,
              });
        }
        if (
          url === `${ASSISTANT_BASE}/conversations/${WORKFLOW_CONVERSATION}` &&
          (init?.method ?? "GET") === "GET"
        ) {
          historyAttempt += 1;
          return jsonResponse(
            historyAttempt === 1
              ? { messages: [], stateVersion: 4 }
              : workflowHistory(4, WORKFLOW_TURN),
          );
        }
        return undefined;
      });
      const transport = new AevatarAssistantTransport();
      const conversation = await transport.createConversation();

      const turn = collectWorkflowTurn(transport, conversation.id, "create");
      await vi.advanceTimersByTimeAsync(600);
      const events = await turn;

      expect(events.at(-1)).toMatchObject({
        event: "turn.completed",
        status: "completed",
      });
      expect(recoveryAttempt).toBe(2);
      expect(historyAttempt).toBe(2);
      expect(streams).toHaveBeenCalledTimes(1);
      expect(
        fetchMock.mock.calls.some(([input]) =>
          String(input).includes("/conversations/create-recovery/"),
        ),
      ).toBe(true);
      expect(
        (await transport.getHistory(conversation.id)).conversation.id,
      ).toBe(WORKFLOW_CONVERSATION);
    } finally {
      vi.useRealTimers();
    }
  });

  it.each([
    [
      "a truncated response",
      {
        completion: {
          kind: "network_error",
          code: "stream_closed",
          message: "truncated",
        } satisfies ChatStreamCompletionResult,
      },
    ],
    [
      "a network failure",
      {
        headers: {
          kind: "network_error",
          code: "network_error",
          message: "offline",
        } satisfies ChatStreamHeadersResult,
      },
    ],
  ])(
    "uses create recovery after %s without replaying the POST",
    async (_label, result) => {
      const streams = mockChatStreams((request) =>
        request.url === WORKFLOW_URL ? result : undefined,
      );
      stubFetch((url, init) => {
        if (url.includes("/conversations/create-recovery/")) {
          return jsonResponse({
            status: "append_committed",
            conversationId: WORKFLOW_CONVERSATION,
            stateVersion: 3,
            turnId: WORKFLOW_TURN,
          });
        }
        if (
          url === `${ASSISTANT_BASE}/conversations/${WORKFLOW_CONVERSATION}` &&
          (init?.method ?? "GET") === "GET"
        ) {
          return jsonResponse(workflowHistory(3, WORKFLOW_TURN));
        }
        return undefined;
      });
      const transport = new AevatarAssistantTransport();
      const conversation = await transport.createConversation();

      const events = await collectWorkflowTurn(
        transport,
        conversation.id,
        "create",
      );

      expect(events.at(-1)).toMatchObject({
        event: "turn.completed",
        status: "completed",
      });
      expect(streams).toHaveBeenCalledTimes(1);
    },
  );

  it("adopts abort-path recovery after RUN_STARTED supplied a run actor id", async () => {
    let markRunStartedDelivered: (() => void) | undefined;
    const runStartedDelivered = new Promise<void>((resolve) => {
      markRunStartedDelivered = resolve;
    });
    const streamSpy = vi
      .spyOn(chatStreamClient, "start")
      .mockImplementation((request): ChatStreamRequestHandle => {
        queueMicrotask(() => {
          request.onFrames([
            { runStarted: { threadId: RUN_ACTOR, runId: RUN_ACTOR } },
          ]);
          markRunStartedDelivered?.();
        });
        return {
          headers: Promise.resolve({
            kind: "response",
            status: 200,
            contentType: "text/event-stream",
          }),
          completion: new Promise<ChatStreamCompletionResult>(() => undefined),
          cancel() {
            // The production client settles this through the abort signal. The
            // transport's independent recovery must not share that signal.
          },
        };
      });
    const fetchMock = stubFetch((url, init) => {
      if (url.includes("/conversations/create-recovery/")) {
        return jsonResponse({
          status: "append_committed",
          conversationId: WORKFLOW_CONVERSATION,
          stateVersion: 3,
          turnId: WORKFLOW_TURN,
        });
      }
      if (
        url === `${ASSISTANT_BASE}/conversations/${WORKFLOW_CONVERSATION}` &&
        (init?.method ?? "GET") === "GET"
      ) {
        return jsonResponse(workflowHistory(3, WORKFLOW_TURN));
      }
      return undefined;
    });
    const transport = new AevatarAssistantTransport();
    const conversation = await transport.createConversation();
    const events: TurnEvent[] = [];
    const handle = transport.sendMessage(conversation.id, "create", (event) => {
      events.push(event);
    });
    await vi.waitFor(() => expect(streamSpy).toHaveBeenCalledTimes(1));
    await runStartedDelivered;

    handle.cancel();

    expect(events.at(-1)).toMatchObject({
      event: "turn.completed",
      status: "cancelled",
    });
    await vi.waitFor(() =>
      expect(
        fetchMock.mock.calls.some(([input]) =>
          String(input).includes("/conversations/create-recovery/"),
        ),
      ).toBe(true),
    );
    await vi.waitFor(async () => {
      expect(
        (await transport.getHistory(conversation.id)).conversation.id,
      ).toBe(WORKFLOW_CONVERSATION);
    });
    expect(
      fetchMock.mock.calls.some(([input, init]) =>
        isTypedCommandRequest(
          String(input),
          init as RequestInit | undefined,
          "task.stop",
        ),
      ),
    ).toBe(false);
  });

  it("rejects a create recovery identity outside the chatc family", async () => {
    mockChatStreams((request) =>
      request.url === WORKFLOW_URL ? {} : undefined,
    );
    stubFetch((url) =>
      url.includes("/conversations/create-recovery/")
        ? jsonResponse({
            status: "append_committed",
            conversationId: CONVERSATION_ID,
            stateVersion: 3,
            turnId: WORKFLOW_TURN,
          })
        : undefined,
    );
    const transport = new AevatarAssistantTransport();
    const conversation = await transport.createConversation();

    const events = await collectWorkflowTurn(
      transport,
      conversation.id,
      "create",
    );

    expect(events.at(-1)).toMatchObject({
      event: "turn.completed",
      status: "failed",
      error: { code: "stream_protocol_error" },
    });
  });

  it("rejects workflow context from a different authenticated scope", async () => {
    const fetchMock = stubFetch(
      routeWorkflow([
        {
          custom: {
            name: "aevatar.chat.context",
            payload: {
              scopeId: "another-user",
              conversationId: WORKFLOW_CONVERSATION,
              turnId: WORKFLOW_TURN,
              stateVersion: "0",
            },
          },
        },
        ...WORKFLOW_TAIL,
      ]),
    );
    const transport = new AevatarAssistantTransport();
    const conversation = await transport.createConversation();

    const events = await collectWorkflowTurn(
      transport,
      conversation.id,
      "create",
    );

    expect(events.at(-1)).toMatchObject({
      event: "turn.completed",
      status: "failed",
      error: { code: "stream_protocol_error" },
    });
    expect(
      fetchMock.mock.calls.some(([input]) =>
        String(input).includes("/conversations/create-recovery/"),
      ),
    ).toBe(false);
  });

  it("accepts workflow context when the local auth user is not hydrated", async () => {
    useAuthStore.getState().setUser(null);
    const fetchMock = stubFetch(
      routeWorkflow([...WORKFLOW_PREAMBLE, ...WORKFLOW_TAIL]),
    );
    const transport = new AevatarAssistantTransport();
    const conversation = await transport.createConversation();

    const events = await collectWorkflowTurn(
      transport,
      conversation.id,
      "create",
    );

    expect(events.at(-1)).toMatchObject({
      event: "turn.completed",
      status: "completed",
    });
    expect(
      fetchMock.mock.calls.some(([input]) =>
        String(input).includes("/conversations/create-recovery/"),
      ),
    ).toBe(false);
  });

  it("fails closed after recovery exhaustion and reuses the command for the same prompt", async () => {
    vi.useFakeTimers();
    try {
      let streamAttempt = 0;
      const streams = mockChatStreams((request) => {
        if (request.url !== WORKFLOW_URL) return undefined;
        streamAttempt += 1;
        return streamAttempt === 1
          ? {}
          : {
              headers: {
                kind: "http_error",
                status: 400,
                body: JSON.stringify({ code: "INVALID", message: "Rejected." }),
              },
            };
      });
      stubFetch((url) =>
        url.includes("/conversations/create-recovery/")
          ? jsonResponse(
              { code: "CREATE_NOT_FOUND", message: "Not ready." },
              404,
            )
          : undefined,
      );
      const transport = new AevatarAssistantTransport();
      const conversation = await transport.createConversation();

      const first = collectWorkflowTurn(
        transport,
        conversation.id,
        "same prompt",
      );
      await vi.advanceTimersByTimeAsync(3_000);
      const firstEvents = await first;
      expect(firstEvents.at(-1)).toMatchObject({
        event: "turn.completed",
        status: "failed",
      });

      const secondEvents = await collectWorkflowTurn(
        transport,
        conversation.id,
        "same prompt",
      );
      expect(secondEvents.at(-1)).toMatchObject({
        event: "turn.completed",
        status: "failed",
      });
      const bodies = streams.mock.calls.map(
        ([request]) => JSON.parse(request.bodyText) as Record<string, unknown>,
      );
      expect(bodies).toHaveLength(2);
      expect(bodies[1]?.["commandId"]).toBe(bodies[0]?.["commandId"]);
    } finally {
      vi.useRealTimers();
    }
  });

  it("remints a workflow session when server history is restored", async () => {
    const SECOND_CONVERSATION = "chatc-11111111111111111111111111111111";
    const sessionIds: string[] = [];
    mockChatStreams((request) => {
      if (request.url !== WORKFLOW_URL) return undefined;
      sessionIds.push(
        String(
          (JSON.parse(request.bodyText) as Record<string, unknown>)[
            "sessionId"
          ],
        ),
      );
      return { frames: [...WORKFLOW_PREAMBLE, ...WORKFLOW_TAIL] };
    });
    stubFetch((url, init) => {
      if (
        url === `${ASSISTANT_BASE}/conversations` &&
        (init?.method ?? "GET") === "GET"
      ) {
        return jsonResponse({
          conversations: [
            { id: WORKFLOW_CONVERSATION, title: "First" },
            { id: SECOND_CONVERSATION, title: "Second" },
          ],
        });
      }
      if (
        url === `${ASSISTANT_BASE}/conversations/${WORKFLOW_CONVERSATION}` &&
        (init?.method ?? "GET") === "GET"
      ) {
        return jsonResponse(workflowHistory(3));
      }
      if (
        url === `${ASSISTANT_BASE}/conversations/${SECOND_CONVERSATION}` &&
        (init?.method ?? "GET") === "GET"
      ) {
        return jsonResponse(workflowHistory(3, "turn-second-seed"));
      }
      return undefined;
    });
    const transport = new AevatarAssistantTransport();
    await transport.listConversations();
    await transport.getHistory(WORKFLOW_CONVERSATION);
    await collectWorkflowTurn(transport, WORKFLOW_CONVERSATION, "first");
    await transport.getHistory(SECOND_CONVERSATION);
    await transport.getHistory(WORKFLOW_CONVERSATION);
    await collectWorkflowTurn(transport, WORKFLOW_CONVERSATION, "second");

    expect(sessionIds).toHaveLength(2);
    expect(sessionIds[1]).not.toBe(sessionIds[0]);
  });

  it("fails closed when a create turn never names its server conversation", async () => {
    stubFetch(
      routeWorkflow([
        {
          timestamp: "1785297207163",
          custom: {
            name: "aevatar.chat.context",
            payload: {
              "@type":
                "type.googleapis.com/aevatar.workflow.runs.WorkflowChatContextPayload",
              scopeId: USER_ID,
              // No conversationId: accepting this would strand the record on
              // its placeholder and mint a second conversation on the next
              // send.
              turnId: WORKFLOW_TURN,
              stateVersion: "3",
            },
          },
        },
        { runStarted: { threadId: RUN_ACTOR, runId: RUN_ACTOR } },
        { runFinished: { threadId: RUN_ACTOR } },
      ]),
    );
    const transport = new AevatarAssistantTransport();
    const conversation = await transport.createConversation();

    const events = await collectWorkflowTurn(transport, conversation.id, "hi");

    expect(events.at(-1)).toMatchObject({
      event: "turn.completed",
      status: "failed",
      error: { code: "stream_protocol_error" },
    });
  });

  it("rejects a replay that moves the turn to another conversation", async () => {
    let attempt = 0;
    mockChatStreams((request) => {
      if (request.url !== WORKFLOW_URL) return undefined;
      attempt += 1;
      // The retry comes back on a different `chatc-…` than the one this run
      // already adopted.
      const conversationId =
        attempt === 1
          ? WORKFLOW_CONVERSATION
          : "chatc-1111111111111111111111111111111";
      return {
        frames: [
          {
            timestamp: "1785297207163",
            custom: {
              name: "aevatar.chat.context",
              payload: {
                "@type":
                  "type.googleapis.com/aevatar.workflow.runs.WorkflowChatContextPayload",
                scopeId: USER_ID,
                conversationId,
                turnId: WORKFLOW_TURN,
                stateVersion: "3",
              },
            },
          },
          { runStarted: { threadId: RUN_ACTOR, runId: RUN_ACTOR } },
          { runFinished: { threadId: RUN_ACTOR } },
        ],
      };
    });
    const transport = new AevatarAssistantTransport();
    const conversation = await transport.createConversation();
    await collectWorkflowTurn(transport, conversation.id, "first");

    // Second turn: the conversation has adopted WORKFLOW_CONVERSATION, and a
    // context frame naming a different one must not silently re-key it.
    const events = await collectWorkflowTurn(transport, conversation.id, "two");

    expect(events.at(-1)).toMatchObject({
      event: "turn.completed",
      status: "failed",
      error: { code: "stream_protocol_error" },
    });
    expect((await transport.getHistory(conversation.id)).conversation.id).toBe(
      WORKFLOW_CONVERSATION,
    );
  });

  it("maps runError to a failed turn with its upstream code", async () => {
    stubFetch(
      routeWorkflow([
        ...WORKFLOW_PREAMBLE,
        { runError: { code: "WORKFLOW_FAILED", message: "engine died" } },
      ]),
    );
    const transport = new AevatarAssistantTransport();
    const conversation = await seedWorkflowConversation(transport);

    const events = await collectWorkflowTurn(transport, conversation.id, "hi");

    const terminal = events[events.length - 1];
    expect(
      terminal?.event === "turn.completed" && {
        status: terminal.status,
        code: terminal.error?.code,
      },
    ).toEqual({ status: "failed", code: "WORKFLOW_FAILED" });
  });

  it("cancels client-side without posting the actor surface's :stop", async () => {
    const encoder = new TextEncoder();
    let preambleSent = false;
    const mock = stubFetch((url, init) => {
      if (url === WORKFLOW_URL && init?.method === "POST") {
        return new Response(
          new ReadableStream<Uint8Array>({
            pull(controller) {
              if (!preambleSent) {
                preambleSent = true;
                controller.enqueue(
                  encoder.encode(
                    WORKFLOW_PREAMBLE.map(
                      (frame) => `data: ${JSON.stringify(frame)}\n\n`,
                    ).join(""),
                  ),
                );
                return;
              }
              // Hang: the run is still executing server-side.
              return new Promise<never>(() => {});
            },
          }),
          { status: 200, headers: { "Content-Type": "text/event-stream" } },
        );
      }
      return undefined;
    });
    const transport = new AevatarAssistantTransport();
    const conversation = await seedWorkflowConversation(transport);

    const events: TurnEvent[] = [];
    const terminal = new Promise<void>((resolve) => {
      const handle = transport.sendMessage(conversation.id, "hi", (event) => {
        events.push(event);
        if (event.event === "turn.completed") resolve();
        if (event.event === "turn.status" && event.status === "running") {
          handle.cancel();
        }
      });
    });
    await terminal;

    expect(
      events.some(
        (event) =>
          event.event === "turn.completed" && event.status === "cancelled",
      ),
    ).toBe(true);
    expect(
      mock.mock.calls.some(([input]) => String(input).endsWith("/stop")),
    ).toBe(false);
  });

  it("does not expose syncing or poll create recovery during an active first turn", async () => {
    vi.useFakeTimers();
    try {
      vi.spyOn(chatStreamClient, "start").mockImplementation(
        (request): ChatStreamRequestHandle => {
          const headers = Promise.resolve({
            kind: "response" as const,
            status: 200,
            contentType: "text/event-stream",
          });
          const completion = headers.then(() => {
            request.onFrames([
              { runStarted: { threadId: RUN_ACTOR, runId: RUN_ACTOR } },
            ]);
            return new Promise<ChatStreamCompletionResult>(() => undefined);
          });
          return { headers, completion, cancel: vi.fn() };
        },
      );
      const mock = stubFetch();
      const transport = new AevatarAssistantTransport();
      const placeholder = await transport.createConversation();
      transport.sendMessage(placeholder.id, "healthy create", () => undefined);
      expect(
        (await transport.getHistory(placeholder.id)).awaitingProjection,
      ).toBeUndefined();
      const internals = workflowInternals(transport);
      await vi.waitFor(() =>
        expect(
          internals.conversations.get(placeholder.id)?.turnState.activeTurn
            ?.status,
        ).toBe("running"),
      );

      const activeHistory = await transport.getHistory(placeholder.id);
      expect(activeHistory.awaitingProjection).toBeUndefined();
      const pendingOutcome = transport.reconcileProjection(placeholder.id);
      await vi.advanceTimersByTimeAsync(2_000);
      expect(
        mock.mock.calls.some(([input]) =>
          String(input).includes("/conversations/create-recovery/"),
        ),
      ).toBe(false);

      useAuthStore.getState().setUser(null);
      await expect(pendingOutcome).resolves.toMatchObject({
        status: "timed_out",
      });
    } finally {
      vi.useRealTimers();
    }
  });

  it("[branch-regression] keeps a new-chat mirror untouched while a follow-up turn is active", async () => {
    let now = 0;
    mockChatStreams((request) =>
      request.url === WORKFLOW_URL
        ? {
            frames: [
              ...WORKFLOW_PREAMBLE,
              { textMessageStart: { messageId: "live-first" } },
              { textMessageContent: { delta: "Local first reply" } },
              { textMessageEnd: {} },
              ...WORKFLOW_TAIL,
            ],
          }
        : undefined,
    );
    stubFetch();
    const transport = new AevatarAssistantTransport(() => now);
    const placeholder = await transport.createConversation();
    await collectWorkflowTurn(transport, placeholder.id, "first prompt");
    now = 20_000;

    const history = await transport.getHistory(placeholder.id);
    const internals = workflowInternals(transport);
    const stored = internals.conversations.get(history.conversation.id)!;
    stored.turnState = {
      ...stored.turnState,
      messages: [
        ...stored.turnState.messages,
        {
          id: "optimistic-follow-up",
          role: "user",
          schema_version: 1,
          blocks: [
            {
              type: "text",
              block_id: "optimistic-follow-up-text",
              text: "follow-up prompt",
            },
          ],
          created_at: new Date(now).toISOString(),
        },
      ],
      activeTurn: { turnId: null, status: "running", error: null },
    };
    const before = stored.turnState.messages;

    const observed = internals.applyHistoryResponse(history.conversation.id, {
      messages: [
        {
          id: "server-first-assistant",
          role: "assistant",
          content: "Local first reply",
          timestamp: 1,
          turnId: WORKFLOW_TURN,
        },
      ],
      stateVersion: 4,
    });

    expect(observed.turnState.messages).toBe(before);
    expect(
      observed.turnState.messages.some(
        (message) => message.id === "optimistic-follow-up",
      ),
    ).toBe(true);
  });

  it("replaces a longer new-chat mirror once the current fence contains its required turn", async () => {
    let now = 0;
    mockChatStreams((request) =>
      request.url === WORKFLOW_URL
        ? {
            frames: [
              ...WORKFLOW_PREAMBLE,
              { textMessageStart: { messageId: "replace-first" } },
              { textMessageContent: { delta: "Local first reply" } },
              { textMessageEnd: {} },
              ...WORKFLOW_TAIL,
            ],
          }
        : undefined,
    );
    stubFetch();
    const transport = new AevatarAssistantTransport(() => now);
    const placeholder = await transport.createConversation();
    await collectWorkflowTurn(transport, placeholder.id, "first prompt");
    now = 20_000;

    const history = await transport.getHistory(placeholder.id);
    const internals = workflowInternals(transport);
    const stored = internals.conversations.get(history.conversation.id)!;
    const before = stored.turnState.messages;
    expect(
      before
        .filter((message) => message.role === "assistant")
        .every((message) => message.turnId === undefined),
    ).toBe(true);
    expect(stored.requiredTurnId).toBe(WORKFLOW_TURN);

    const observed = internals.applyHistoryResponse(history.conversation.id, {
      messages: [
        {
          id: "server-first-assistant",
          role: "assistant",
          content: "Persisted first reply",
          timestamp: 1,
          turnId: WORKFLOW_TURN,
        },
      ],
      stateVersion: 4,
    });

    expect(observed.turnState.messages).not.toBe(before);
    expect(observed.turnState.messages.length).toBeLessThan(before.length);
    expect(observed.turnState.messages[0]?.id).toBe("server-first-assistant");
    expect(
      observed.turnState.messages.some((message) =>
        message.id.startsWith("assistant-activity-"),
      ),
    ).toBe(true);
  });

  it("[branch-regression] keeps a longer new-chat mirror when the current fence omits its required turn", async () => {
    let now = 0;
    mockChatStreams((request) =>
      request.url === WORKFLOW_URL
        ? {
            frames: [
              ...WORKFLOW_PREAMBLE,
              { textMessageStart: { messageId: "missing-first" } },
              { textMessageContent: { delta: "Local first reply" } },
              { textMessageEnd: {} },
              ...WORKFLOW_TAIL,
            ],
          }
        : undefined,
    );
    stubFetch();
    const transport = new AevatarAssistantTransport(() => now);
    const placeholder = await transport.createConversation();
    await collectWorkflowTurn(transport, placeholder.id, "first prompt");
    now = 20_000;

    const history = await transport.getHistory(placeholder.id);
    const internals = workflowInternals(transport);
    const stored = internals.conversations.get(history.conversation.id)!;
    const before = stored.turnState.messages;
    expect(
      before
        .filter((message) => message.role === "assistant")
        .every((message) => message.turnId === undefined),
    ).toBe(true);

    const observed = internals.applyHistoryResponse(history.conversation.id, {
      messages: [
        {
          id: "older-assistant",
          role: "assistant",
          content: "Older reply",
          timestamp: 1,
          turnId: "turn-before-the-new-chat",
        },
      ],
      stateVersion: 4,
    });

    expect(observed.turnState.messages).toBe(before);
  });

  it("[guard] keeps below-fence and legacy shorter reads from replacing local", async () => {
    let now = 0;
    mockChatStreams((request) =>
      request.url === WORKFLOW_URL
        ? {
            frames: [
              ...WORKFLOW_PREAMBLE,
              { textMessageStart: { messageId: "guard-first" } },
              { textMessageContent: { delta: "Local first reply" } },
              { textMessageEnd: {} },
              ...WORKFLOW_TAIL,
            ],
          }
        : undefined,
    );
    stubFetch();
    const transport = new AevatarAssistantTransport(() => now);
    const placeholder = await transport.createConversation();
    await collectWorkflowTurn(transport, placeholder.id, "first prompt");
    now = 20_000;

    const history = await transport.getHistory(placeholder.id);
    const internals = workflowInternals(transport);
    const stored = internals.conversations.get(history.conversation.id)!;
    const before = stored.turnState.messages;
    const entry = {
      id: "server-first-assistant",
      role: "assistant",
      content: "Persisted first reply",
      timestamp: 1,
      turnId: WORKFLOW_TURN,
    };

    expect(
      internals.applyHistoryResponse(history.conversation.id, {
        messages: [entry],
        stateVersion: 2,
      }).turnState.messages,
    ).toBe(before);
    expect(
      internals.applyHistoryResponse(history.conversation.id, [entry]).turnState
        .messages,
    ).toBe(before);
  });

  it("deletes a legacy workflow conversation through its server id", async () => {
    const mock = stubFetch(
      routeWorkflow([
        ...WORKFLOW_PREAMBLE,
        { textMessageStart: { messageId: "wm-1" } },
        { textMessageContent: { delta: "First." } },
        { textMessageEnd: {} },
        ...WORKFLOW_TAIL,
      ]),
      (_url, init) =>
        init?.method === "DELETE" ? jsonResponse({}) : undefined,
    );
    const transport = new AevatarAssistantTransport();
    const conversation = await seedWorkflowConversation(transport);
    await collectWorkflowTurn(transport, conversation.id, "first");

    await transport.deleteConversation(conversation.id);

    expect(
      mock.mock.calls.some(
        ([input, init]) =>
          String(input) ===
            `${ASSISTANT_BASE}/conversations/${WORKFLOW_CONVERSATION}` &&
          init?.method === "DELETE",
      ),
    ).toBe(true);
    await expect(transport.getHistory(conversation.id)).rejects.toBeInstanceOf(
      AssistantConversationNotFoundError,
    );
    await expect(
      transport.getHistory(WORKFLOW_CONVERSATION),
    ).rejects.toBeInstanceOf(AssistantConversationNotFoundError);
  });

  it("[branch-regression] deletes an aliased placeholder through the wire when its receipt is gone", async () => {
    mockChatStreams((request) =>
      request.url === WORKFLOW_URL
        ? { frames: [...WORKFLOW_PREAMBLE, ...WORKFLOW_TAIL] }
        : undefined,
    );
    const mock = stubFetch((_url, init) =>
      init?.method === "DELETE" ? jsonResponse({}) : undefined,
    );
    const transport = new AevatarAssistantTransport();
    const placeholder = await transport.createConversation();
    await collectWorkflowTurn(transport, placeholder.id, "create");
    const receipt = findReceiptByPlaceholder(placeholder.id);
    expect(receipt?.conversationId).toBe(WORKFLOW_CONVERSATION);
    deleteReceipt(receipt!.commandId);

    await transport.deleteConversation(placeholder.id);

    expect(
      mock.mock.calls.some(
        ([input, init]) =>
          String(input) ===
            `${ASSISTANT_BASE}/conversations/${WORKFLOW_CONVERSATION}` &&
          init?.method === "DELETE",
      ),
    ).toBe(true);
  });

  it("recovers and deletes a create removed before its context can arrive", async () => {
    const headers = new Promise<ChatStreamHeadersResult>(() => undefined);
    const completion = new Promise<ChatStreamCompletionResult>(() => undefined);
    vi.spyOn(chatStreamClient, "start").mockReturnValue({
      headers,
      completion,
      cancel: vi.fn(),
    });
    const mock = stubFetch((url, init) => {
      if (url.includes("/conversations/create-recovery/")) {
        return jsonResponse({
          status: "append_committed",
          conversationId: WORKFLOW_CONVERSATION,
          stateVersion: 3,
          turnId: WORKFLOW_TURN,
        });
      }
      if (
        url === `${ASSISTANT_BASE}/conversations/${WORKFLOW_CONVERSATION}` &&
        init?.method === "DELETE"
      ) {
        return jsonResponse({});
      }
      if (url === `${ASSISTANT_BASE}/conversations`) {
        return jsonResponse({
          conversations: [{ id: WORKFLOW_CONVERSATION, title: "Stale row" }],
        });
      }
      return undefined;
    });
    const transport = new AevatarAssistantTransport();
    const placeholder = await transport.createConversation();
    transport.sendMessage(placeholder.id, "create then delete", () => undefined);
    await vi.waitFor(() => expect(chatStreamClient.start).toHaveBeenCalledOnce());

    await transport.deleteConversation(placeholder.id);
    await vi.waitFor(() =>
      expect(
        mock.mock.calls.some(
          ([input, init]) =>
            String(input) ===
              `${ASSISTANT_BASE}/conversations/${WORKFLOW_CONVERSATION}` &&
            init?.method === "DELETE",
        ),
      ).toBe(true),
    );

    expect(await transport.listConversations()).toEqual([]);
    expect(listDeletionIntents()).toEqual([]);
  });

  it("posts a workflow action continuation to the actor named by its frame", async () => {
    const actionBodies: Record<string, unknown>[] = [];
    const mock = stubFetch(
      routeWorkflow([
        ...WORKFLOW_PREAMBLE,
        actionRequestFrame({
          actorId: ACTION_ACTOR,
          originTurnId: WORKFLOW_TURN,
        }),
        { runFinished: { threadId: RUN_ACTOR, status: "blocked" } },
        { stateSnapshot: { snapshot: { actorId: RUN_ACTOR } } },
      ]),
      (url, init) => {
        if (
          url !== `${ASSISTANT_BASE}/conversations/${ACTION_ACTOR}/stream` ||
          init?.method !== "POST"
        ) {
          return undefined;
        }
        actionBodies.push(
          JSON.parse(String(init.body)) as Record<string, unknown>,
        );
        return sseResponse([
          {
            type: "RUN_STARTED",
            actorId: ACTION_ACTOR,
            turnId: "turn-action-continuation-1",
          },
          { type: "RUN_FINISHED" },
        ]);
      },
    );
    const transport = new AevatarAssistantTransport();
    const conversation = await seedWorkflowConversation(transport);
    await collectWorkflowTurn(transport, conversation.id, "Connect GitHub");

    const events: TurnEvent[] = [];
    await new Promise<void>((resolve) => {
      const handle = transport.continueActions(
        conversation.id,
        WORKFLOW_TURN,
        [
          {
            actionRequestId: "act-action-1",
            originTurnId: WORKFLOW_TURN,
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

    expect(
      mock.mock.calls.some(
        ([input]) =>
          String(input).includes(
            `/conversations/${WORKFLOW_CONVERSATION}/stream`,
          ) ||
          String(input).includes(`/conversations/${conversation.id}/stream`),
      ),
    ).toBe(false);
    expect(
      mock.mock.calls
        .filter(([input]) => String(input) === WORKFLOW_URL)
        .map(([, init]) => JSON.parse(String(init?.body)) as { type?: string })
        .some((body) => body.type === "action.continue"),
    ).toBe(false);
    expect(actionBodies).toHaveLength(1);
    expect(actionBodies[0]).toMatchObject({
      type: "action.continue",
      originTurnId: WORKFLOW_TURN,
      actions: [
        {
          actionRequestId: "act-action-1",
          originTurnId: WORKFLOW_TURN,
          disposition: "completed",
          resource: {
            userService: {
              userServiceId: "00000000-0000-4000-8000-000000000123",
            },
          },
        },
      ],
    });
    expect(
      events.some(
        (event) =>
          event.event === "block.updated" &&
          "outcome_note" in event.patch &&
          event.patch.outcome_note ===
            "Reported — awaiting assistant verification.",
      ),
    ).toBe(true);
  });
});

describe("projection lifecycle boundaries", () => {
  const CHAT_ID = "chatc-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

  it("clears local mirrors on account switch but preserves same-user refreshes", async () => {
    stubFetch((url) =>
      url === `${ASSISTANT_BASE}/conversations`
        ? jsonResponse({ conversations: [] })
        : undefined,
    );
    const transport = new AevatarAssistantTransport();
    const conversation = await transport.createConversation();

    useAuthStore.getState().setUser({ id: USER_ID } as User);
    expect((await transport.listConversations()).map(({ id }) => id)).toContain(
      conversation.id,
    );

    useAuthStore.getState().setUser({ id: "another-user" } as User);
    expect(await transport.listConversations()).toEqual([]);
    await expect(transport.getHistory(conversation.id)).rejects.toBeInstanceOf(
      AssistantConversationNotFoundError,
    );
  });

  it("settles an abandoned reconciliation when the account scope changes", async () => {
    stubFetch((url) =>
      url === `${ASSISTANT_BASE}/conversations`
        ? jsonResponse({ conversations: [{ id: CHAT_ID }] })
        : undefined,
    );
    const transport = new AevatarAssistantTransport();
    await transport.getHistory(CHAT_ID);
    const outcome = transport.reconcileProjection(CHAT_ID);

    useAuthStore.getState().setUser({ id: "another-user" } as User);

    await expect(outcome).resolves.toEqual({
      status: "timed_out",
      conversationId: CHAT_ID,
    });
  });

  it("[branch-regression] reads a cold canonical receipt from the transcript before serving pending", async () => {
    recordCreateReceipt("cold-command", "workflow-pending-cold");
    adoptReceiptIdentity("cold-command", CHAT_ID, 3);
    const mock = stubFetch((url, init) =>
      url === `${ASSISTANT_BASE}/conversations/${CHAT_ID}` &&
      (init?.method ?? "GET") === "GET"
        ? jsonResponse({
            messages: [
              {
                id: "cold-assistant",
                role: "assistant",
                content: "Already materialized",
                timestamp: 1,
                turnId: "cold-turn",
              },
            ],
            stateVersion: 3,
          })
        : undefined,
    );
    const transport = new AevatarAssistantTransport();

    const history = await transport.getHistory(CHAT_ID);

    expect(history.messages[0]?.id).toBe("cold-assistant");
    expect(history.awaitingProjection).toBeUndefined();
    expect(
      mock.mock.calls.filter(
        ([input]) =>
          String(input) === `${ASSISTANT_BASE}/conversations/${CHAT_ID}`,
      ),
    ).toHaveLength(1);
  });

  it("sweeps a persisted deletion intent after a transport reload", async () => {
    recordDeletionIntent(
      "reload-delete-command",
      "workflow-pending-reload-delete",
    );
    const mock = stubFetch((url, init) => {
      if (url.includes("/conversations/create-recovery/")) {
        return jsonResponse({
          status: "append_committed",
          conversationId: CHAT_ID,
          stateVersion: 3,
          turnId: "reload-delete-turn",
        });
      }
      if (
        url === `${ASSISTANT_BASE}/conversations/${CHAT_ID}` &&
        init?.method === "DELETE"
      ) {
        return jsonResponse({});
      }
      if (url === `${ASSISTANT_BASE}/conversations`) {
        return jsonResponse({ conversations: [{ id: CHAT_ID }] });
      }
      return undefined;
    });
    const reloadedTransport = new AevatarAssistantTransport();

    await reloadedTransport.listConversations();
    await vi.waitFor(() => expect(listDeletionIntents()).toEqual([]));

    expect(
      mock.mock.calls.some(
        ([input, init]) =>
          String(input) === `${ASSISTANT_BASE}/conversations/${CHAT_ID}` &&
          init?.method === "DELETE",
      ),
    ).toBe(true);
    expect(await reloadedTransport.listConversations()).toEqual([]);
  });

  it("uses raw index membership as cold 404 evidence and then materializes", async () => {
    let projected = false;
    const mock = stubFetch((url, init) => {
      if (url === `${ASSISTANT_BASE}/conversations`) {
        return jsonResponse({ conversations: [{ id: CHAT_ID, title: "New" }] });
      }
      if (
        url === `${ASSISTANT_BASE}/conversations/${CHAT_ID}` &&
        (init?.method ?? "GET") === "GET"
      ) {
        return projected
          ? jsonResponse({
              messages: [
                {
                  id: "assistant-turn-a",
                  role: "assistant",
                  content: "Ready",
                  timestamp: 1,
                  turnId: "turn-a",
                },
              ],
              stateVersion: 1,
            })
          : jsonResponse({ message: "not ready" }, 404);
      }
      return undefined;
    });
    const transport = new AevatarAssistantTransport();

    const pending = await transport.getHistory(CHAT_ID);
    expect(pending.awaitingProjection).toBe(true);
    projected = true;
    await expect(transport.reconcileProjection(CHAT_ID)).resolves.toEqual({
      status: "materialized",
      conversationId: CHAT_ID,
    });
    expect((await transport.getHistory(CHAT_ID)).messages[0]?.id).toBe(
      "assistant-turn-a",
    );
    expect(
      mock.mock.calls.filter(
        ([input]) => String(input) === `${ASSISTANT_BASE}/conversations`,
      ),
    ).toHaveLength(1);
  });

  it("single-flights concurrent projection waiters", async () => {
    let projected = false;
    stubFetch((url) => {
      if (url === `${ASSISTANT_BASE}/conversations`) {
        return jsonResponse({ conversations: [{ id: CHAT_ID }] });
      }
      if (url === `${ASSISTANT_BASE}/conversations/${CHAT_ID}`) {
        return projected
          ? jsonResponse({ messages: [], stateVersion: 1 })
          : jsonResponse({ message: "not ready" }, 404);
      }
      return undefined;
    });
    const transport = new AevatarAssistantTransport();
    await transport.getHistory(CHAT_ID);
    projected = true;

    const first = transport.reconcileProjection(CHAT_ID);
    const second = transport.reconcileProjection(CHAT_ID);

    expect(second).toBe(first);
    await expect(first).resolves.toMatchObject({ status: "materialized" });
  });

  it("turns a deadline with continuing index membership into stalled provenance", async () => {
    vi.useFakeTimers();
    let now = 0;
    try {
      stubFetch((url) =>
        url === `${ASSISTANT_BASE}/conversations`
          ? jsonResponse({ conversations: [{ id: CHAT_ID }] })
          : undefined,
      );
      const transport = new AevatarAssistantTransport(() => now, () => 0);
      const pending = await transport.getHistory(CHAT_ID);
      expect(pending.awaitingProjection).toBe(true);

      const outcome = transport.reconcileProjection(CHAT_ID);
      await vi.advanceTimersByTimeAsync(0);
      now = 90_000;
      await vi.advanceTimersByTimeAsync(250);

      await expect(outcome).resolves.toMatchObject({ status: "timed_out" });
      expect((await transport.getHistory(CHAT_ID)).projectionStalled).toBe(true);
    } finally {
      vi.useRealTimers();
    }
  });

  it("rejects a cold 404 after one raw absent confirmation", async () => {
    const mock = stubFetch((url) =>
      url === `${ASSISTANT_BASE}/conversations`
        ? jsonResponse({ conversations: [] })
        : undefined,
    );
    const transport = new AevatarAssistantTransport();

    await expect(transport.getHistory(CHAT_ID)).rejects.toBeInstanceOf(
      AssistantConversationNotFoundError,
    );
    expect(mock).toHaveBeenCalledTimes(2);
  });

  it("deletes a dispatched unaliased create locally without a placeholder DELETE", async () => {
    const headers = new Promise<ChatStreamHeadersResult>(() => undefined);
    const completion = new Promise<ChatStreamCompletionResult>(() => undefined);
    const start = vi.spyOn(chatStreamClient, "start").mockReturnValue({
      headers,
      completion,
      cancel: vi.fn(),
    });
    const mock = stubFetch((url) =>
      url.includes("/conversations/create-recovery/")
        ? jsonResponse({ message: "not ready" }, 404)
        : undefined,
    );
    const transport = new AevatarAssistantTransport();
    const conversation = await transport.createConversation();
    transport.sendMessage(conversation.id, "Create then delete", () => undefined);
    await vi.waitFor(() => expect(start).toHaveBeenCalledOnce());

    await transport.deleteConversation(conversation.id);

    expect(listDeletionIntents()).toHaveLength(1);
    expect(
      mock.mock.calls.some(
        ([input, init]) =>
          String(input).includes(conversation.id) && init?.method === "DELETE",
      ),
    ).toBe(false);
    await expect(
      transport.getHistory(conversation.id),
    ).rejects.toBeInstanceOf(AssistantConversationNotFoundError);
  });
});

describe("workflow conversations fail approvals honestly", () => {
  // `:approve` addresses a nyxid-chat ACTOR; a workflow run resumes through
  // `runs/{runId}:resume`, which the mount does not proxy. The card can
  // still render (the workflow mapper emits `aevatar.tool_approval.pending`),
  // so the decision must fail with a legible message instead of a 404.
  it("refuses to post the actor approve route for a chatc conversation", async () => {
    const workflowConversation = "chatc-8bd999c402fb37d60cdcd81e3b78cfd";
    const mock = stubFetch((url, init) =>
      url === `${ASSISTANT_BASE}/conversations` &&
      (init?.method ?? "GET") === "GET"
        ? jsonResponse({
            conversations: [
              {
                id: workflowConversation,
                title: "Legacy workflow conversation",
              },
            ],
          })
        : undefined,
    );
    const transport = new AevatarAssistantTransport();
    const conversation = (await transport.listConversations()).find(
      (candidate) => candidate.id === workflowConversation,
    );
    expect(conversation).toBeDefined();

    await expect(
      transport.decideApproval(workflowConversation, "block-1", true),
    ).rejects.toThrow(/Approvals cannot be decided from this chat yet/);
    expect(
      mock.mock.calls.some(([input]) => String(input).endsWith("/approve")),
    ).toBe(false);
  });
});
