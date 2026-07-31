import { act, renderHook } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { PropsWithChildren } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useSendMessage, assistantKeys } from "@/hooks/use-assistant";
import { AevatarAssistantTransport } from "@/lib/assistant/aevatar-transport";
import {
  chatStreamClient,
  type ChatStreamRequest,
  type ChatStreamRequestHandle,
} from "@/lib/assistant/chat-stream-worker-client";
import type { ChatStreamFrame } from "@/lib/assistant/chat-stream-worker-protocol";
import { assistantTransport } from "@/lib/assistant/transport";
import type {
  ConversationHistory,
  TurnEpisode,
  TurnEvent,
} from "@/types/assistant";

const SERVER_CONVERSATION = "chatc-8bd999c402fb37d60cdcd81e3b78cfd";
const TURN_ID = "turn-d619940adcd817c4aeb5d1c3e57f1ca5";
const RUN_ACTOR = "workflow-definition:studio:run:probe";
const HISTORY_URL = `/api/v1/assistant/conversations/${SERVER_CONVERSATION}`;

type HistoryMode = "missing" | "materialized";

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

function serverHistory(assistantText: string) {
  return {
    messages: [
      {
        id: `${TURN_ID}:user`,
        role: "user",
        content: "Connect GitHub",
        timestamp: 1785297207000,
        turnId: TURN_ID,
      },
      {
        id: `${TURN_ID}:assistant`,
        role: "assistant",
        content: assistantText,
        timestamp: 1785297208000,
        turnId: TURN_ID,
        status: "blocked",
      },
    ],
    stateVersion: 4,
  };
}

function workflowContextFrame(): ChatStreamFrame {
  return {
    custom: {
      name: "aevatar.chat.context",
      payload: {
        "@type":
          "type.googleapis.com/aevatar.workflow.runs.WorkflowChatContextPayload",
        scopeId: "user-probe",
        conversationId: SERVER_CONVERSATION,
        turnId: TURN_ID,
        stateVersion: "3",
      },
    },
  };
}

function actionRequestFrame(): ChatStreamFrame {
  return {
    type: "CUSTOM",
    custom: {
      name: "nyxid.action.request",
      payload: {
        schemaVersion: 4,
        actorId: "nyxid-chat-workflow-action-probe",
        originTurnId: TURN_ID,
        taskId: "task-probe",
        stepId: "step-probe",
        actionRequestId: "action-probe",
        action: "service.connect",
        params: {
          catalogService: {
            serviceSlug: "api-github",
            requestedScopes: ["repo"],
          },
        },
      },
    },
  };
}

function hasActionCard(history: ConversationHistory | undefined): boolean {
  return (
    history?.messages.some((message) =>
      message.blocks.some((block) => block.type === "action_card"),
    ) ?? false
  );
}

function createHarness() {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  });
  const Wrapper = ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
  return { queryClient, Wrapper };
}

interface ManualStream {
  request: ChatStreamRequest | null;
  emit(frames: readonly ChatStreamFrame[]): void;
  finish(): void;
}

function installManualStream(): ManualStream {
  let request: ChatStreamRequest | null = null;
  let settle: ((result: { readonly kind: "complete" }) => void) | undefined;
  vi.spyOn(chatStreamClient, "start").mockImplementation((nextRequest) => {
    request = nextRequest;
    const completion = new Promise<{ readonly kind: "complete" }>((resolve) => {
      settle = resolve;
    });
    return {
      headers: Promise.resolve({
        kind: "response",
        status: 200,
        contentType: "text/event-stream",
      }),
      completion,
      cancel: vi.fn(),
    } satisfies ChatStreamRequestHandle;
  });
  return {
    get request() {
      return request;
    },
    emit(frames) {
      if (!request) throw new Error("The workflow stream has not started.");
      request.onFrames(frames);
    },
    finish() {
      if (!settle) throw new Error("The workflow stream has not started.");
      settle({ kind: "complete" });
    },
  };
}

function bindHookTransport(
  transport: AevatarAssistantTransport,
  observedEvents: TurnEvent[],
): void {
  vi.spyOn(assistantTransport, "getHistory").mockImplementation((id) =>
    transport.getHistory(id),
  );
  vi.spyOn(assistantTransport, "listConversations").mockImplementation(() =>
    transport.listConversations(),
  );
  vi.spyOn(assistantTransport, "sendMessage").mockImplementation(
    (id, content, onEvent) =>
      transport.sendMessage(id, content, (event) => {
        observedEvents.push(event);
        onEvent(event);
      }),
  );
  vi.spyOn(assistantTransport, "cancelActiveTurn").mockImplementation((id) =>
    transport.cancelActiveTurn(id),
  );
}

interface ProbeSession {
  readonly transport: AevatarAssistantTransport;
  readonly placeholderId: string;
  readonly stream: ManualStream;
  readonly queryClient: QueryClient;
  readonly observedEvents: TurnEvent[];
  readonly cacheSawCardWhileRunning: () => boolean;
  readonly advanceHistoryClock: (milliseconds: number) => void;
  readonly setHistoryMode: (mode: HistoryMode) => void;
  readonly setServerAssistantText: (text: string) => void;
  readonly unmount: () => void;
}

async function startProbe(): Promise<ProbeSession> {
  let historyMode: HistoryMode = "missing";
  let serverAssistantText =
    "Complete the requested action in NyxID, then continue this conversation.";
  let now = Date.parse("2026-07-31T13:00:00.000Z");
  vi.stubGlobal(
    "fetch",
    vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      if (
        url === "/api/v1/assistant/conversations" &&
        (init?.method ?? "GET") === "GET"
      ) {
        return Promise.resolve(jsonResponse({ conversations: [] }));
      }
      if (url === HISTORY_URL && (init?.method ?? "GET") === "GET") {
        return Promise.resolve(
          historyMode === "missing"
            ? jsonResponse(
                { error: "not_found", error_code: -1, message: "404" },
                404,
              )
            : jsonResponse(serverHistory(serverAssistantText)),
        );
      }
      return Promise.resolve(
        jsonResponse(
          { error: "not_found", error_code: -1, message: "404" },
          404,
        ),
      );
    }),
  );

  const transport = new AevatarAssistantTransport(() => now);
  const conversation = await transport.createConversation();
  const observedEvents: TurnEvent[] = [];
  bindHookTransport(transport, observedEvents);
  const stream = installManualStream();
  const { queryClient, Wrapper } = createHarness();
  let cacheSawCardWhileRunning = false;
  const unsubscribe = queryClient.getQueryCache().subscribe(() => {
    const history = queryClient.getQueryData<ConversationHistory>(
      assistantKeys.history(conversation.id),
    );
    const terminalSeen = observedEvents.some(
      (event) => event.event === "turn.completed",
    );
    if (!terminalSeen && hasActionCard(history)) {
      cacheSawCardWhileRunning = true;
    }
  });
  const hook = renderHook(() => useSendMessage(conversation.id), {
    wrapper: Wrapper,
  });

  await act(async () => {
    await hook.result.current.mutateAsync("Connect GitHub");
  });
  await vi.waitFor(() => expect(stream.request).not.toBeNull());

  return {
    transport,
    placeholderId: conversation.id,
    stream,
    queryClient,
    observedEvents,
    cacheSawCardWhileRunning: () => cacheSawCardWhileRunning,
    advanceHistoryClock: (milliseconds) => {
      now += milliseconds;
    },
    setHistoryMode: (mode) => {
      historyMode = mode;
    },
    setServerAssistantText: (text) => {
      serverAssistantText = text;
    },
    unmount: () => {
      unsubscribe();
      hook.unmount();
      queryClient.clear();
    },
  };
}

async function finishAndWait(session: ProbeSession): Promise<void> {
  await act(async () => {
    session.stream.finish();
  });
  await vi.waitFor(() => {
    expect(
      session.observedEvents.some(
        (event) => event.event === "turn.completed",
      ),
    ).toBe(true);
  });
  await vi.waitFor(() => {
    const episode = session.queryClient.getQueryData<TurnEpisode | null>(
      assistantKeys.episode(session.placeholderId),
    );
    expect(episode?.projecting).toBe(false);
  });
}

async function switchRead(session: ProbeSession): Promise<ConversationHistory> {
  session.queryClient.removeQueries({
    queryKey: assistantKeys.history(session.placeholderId),
  });
  const history = await session.transport.getHistory(session.placeholderId);
  session.queryClient.setQueryData(
    assistantKeys.history(session.placeholderId),
    history,
  );
  return history;
}

function mirrorMintedCard(events: readonly TurnEvent[]): boolean {
  return events.some(
    (event) =>
      event.event === "block.started" && event.block.type === "action_card",
  );
}

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("workflow action-card projection probes", () => {
  it("S1 preserves a card when streamed text makes the local transcript longer", async () => {
    const session = await startProbe();
    session.stream.emit([
      workflowContextFrame(),
      { runStarted: { threadId: RUN_ACTOR, runId: RUN_ACTOR } },
      { textMessageStart: { messageId: "message-s1", role: "assistant" } },
      {
        textMessageContent: {
          messageId: "message-s1",
          delta: "Connect GitHub to continue.",
        },
      },
      { textMessageEnd: { messageId: "message-s1" } },
      actionRequestFrame(),
      { runFinished: { threadId: RUN_ACTOR, status: "completed" } },
    ]);
    session.setHistoryMode("materialized");
    await finishAndWait(session);

    expect(mirrorMintedCard(session.observedEvents)).toBe(true);
    expect(session.cacheSawCardWhileRunning()).toBe(false);
    expect(
      hasActionCard(
        session.queryClient.getQueryData(
          assistantKeys.history(session.placeholderId),
        ),
      ),
    ).toBe(true);
    const withinGrace = await switchRead(session);
    expect(hasActionCard(withinGrace)).toBe(true);
    expect(withinGrace.messages.map((message) => message.id)).not.toContain(
      `${TURN_ID}:assistant`,
    );
    session.unmount();
  });

  it("S2 preserves a card-only blocked turn while adopting materialized text after grace", async () => {
    const session = await startProbe();
    session.stream.emit([
      workflowContextFrame(),
      { runStarted: { threadId: RUN_ACTOR, runId: RUN_ACTOR } },
      actionRequestFrame(),
      { runFinished: { threadId: RUN_ACTOR, status: "blocked" } },
    ]);
    session.setHistoryMode("materialized");
    await finishAndWait(session);

    expect(mirrorMintedCard(session.observedEvents)).toBe(true);
    expect(session.cacheSawCardWhileRunning()).toBe(false);
    expect(
      hasActionCard(
        session.queryClient.getQueryData(
          assistantKeys.history(session.placeholderId),
        ),
      ),
    ).toBe(true);
    const withinGrace = await switchRead(session);
    expect(hasActionCard(withinGrace)).toBe(true);
    expect(withinGrace.messages.map((message) => message.id)).not.toContain(
      `${TURN_ID}:assistant`,
    );

    session.advanceHistoryClock(15_001);
    const materialized = await switchRead(session);
    expect(hasActionCard(materialized)).toBe(true);
    expect(materialized.messages.map((message) => message.id)).toContain(
      `${TURN_ID}:assistant`,
    );
    expect(
      materialized.messages.find(
        (message) => message.id === `${TURN_ID}:assistant`,
      ),
    ).toMatchObject({ turnId: TURN_ID, status: "blocked" });

    session.setServerAssistantText("The server transcript caught up.");
    const converged = await switchRead(session);
    expect(hasActionCard(converged)).toBe(true);
    expect(
      converged.messages
        .flatMap((message) => message.blocks)
        .find((block) => block.block_id === `${TURN_ID}:assistant-text`),
    ).toMatchObject({
      type: "text",
      text: "The server transcript caught up.",
    });
    session.unmount();
  });

  it("S3 keeps a card through a terminal 404 and later materialization", async () => {
    const session = await startProbe();
    session.stream.emit([
      workflowContextFrame(),
      { runStarted: { threadId: RUN_ACTOR, runId: RUN_ACTOR } },
      actionRequestFrame(),
      { runFinished: { threadId: RUN_ACTOR, status: "blocked" } },
    ]);
    await finishAndWait(session);

    expect(mirrorMintedCard(session.observedEvents)).toBe(true);
    expect(
      hasActionCard(
        session.queryClient.getQueryData(
          assistantKeys.history(session.placeholderId),
        ),
      ),
    ).toBe(true);
    session.setHistoryMode("materialized");
    expect(hasActionCard(await switchRead(session))).toBe(true);
    session.advanceHistoryClock(15_001);
    expect(hasActionCard(await switchRead(session))).toBe(true);
    session.unmount();
  });

  it.each(["completed", "blocked"] as const)(
    "S4 discards an action request after a %s terminal",
    async (status) => {
      const session = await startProbe();
      session.stream.emit([
        workflowContextFrame(),
        { runStarted: { threadId: RUN_ACTOR, runId: RUN_ACTOR } },
        { runFinished: { threadId: RUN_ACTOR, status } },
        actionRequestFrame(),
      ]);
      session.setHistoryMode("materialized");
      await finishAndWait(session);

      expect(mirrorMintedCard(session.observedEvents)).toBe(false);
      expect(
        hasActionCard(
          session.queryClient.getQueryData(
            assistantKeys.history(session.placeholderId),
          ),
        ),
      ).toBe(false);
      expect(hasActionCard(await switchRead(session))).toBe(false);
      session.unmount();
    },
  );

  it("S5 preserves the upstream empty text shell around an action terminal", async () => {
    const session = await startProbe();
    session.stream.emit([
      workflowContextFrame(),
      { runStarted: { threadId: RUN_ACTOR, runId: RUN_ACTOR } },
      { textMessageStart: { messageId: TURN_ID, role: "assistant" } },
      actionRequestFrame(),
      { textMessageEnd: { messageId: TURN_ID } },
      { runFinished: { threadId: RUN_ACTOR, status: "blocked" } },
    ]);
    session.setHistoryMode("materialized");
    await finishAndWait(session);

    expect(mirrorMintedCard(session.observedEvents)).toBe(true);
    expect(
      hasActionCard(
        session.queryClient.getQueryData(
          assistantKeys.history(session.placeholderId),
        ),
      ),
    ).toBe(true);
    expect(hasActionCard(await switchRead(session))).toBe(true);
    session.unmount();
  });
});
