import { act, renderHook } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { PropsWithChildren } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  useConversation,
  useSendMessage,
  assistantKeys,
} from "@/hooks/use-assistant";
import { AevatarAssistantTransport } from "@/lib/assistant/aevatar-transport";
import {
  chatStreamClient,
  type ChatStreamRequest,
  type ChatStreamRequestHandle,
} from "@/lib/assistant/chat-stream-worker-client";
import type { ChatStreamFrame } from "@/lib/assistant/chat-stream-worker-protocol";
import { assistantTransport } from "@/lib/assistant/transport";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
import type {
  ConversationHistory,
  TurnEpisode,
  TurnEvent,
} from "@/types/assistant";

const SERVER_CONVERSATION = "chatc-8bd999c402fb37d60cdcd81e3b78cfd";
const TURN_ID = "turn-d619940adcd817c4aeb5d1c3e57f1ca5";
const SECOND_TURN_ID = "turn-261a6458c9b647e99d91a99697115385";
const RUN_ACTOR = "workflow-definition:studio:run:probe";
const HISTORY_URL = `/api/v1/assistant/conversations/${SERVER_CONVERSATION}`;

type HistoryMode = "missing" | "materialized";

interface ServerHistoryMessage {
  readonly id: string;
  readonly role: "user" | "assistant";
  readonly content: string;
  readonly timestamp: number;
  readonly turnId: string;
  readonly status?: "blocked";
}

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

function serverTurnMessages(
  turnId: string,
  userText: string,
  assistantText: string,
  timestamp: number,
): readonly ServerHistoryMessage[] {
  return [
    {
      id: `${turnId}:user`,
      role: "user",
      content: userText,
      timestamp,
      turnId,
    },
    {
      id: `${turnId}:assistant`,
      role: "assistant",
      content: assistantText,
      timestamp: timestamp + 1_000,
      turnId,
      status: "blocked",
    },
  ];
}

function serverHistory(messages: readonly ServerHistoryMessage[]) {
  return {
    messages,
    stateVersion: 100,
  };
}

function workflowContextFrame(
  turnId = TURN_ID,
  stateVersion = 3,
): ChatStreamFrame {
  return {
    custom: {
      name: "aevatar.chat.context",
      payload: {
        "@type":
          "type.googleapis.com/aevatar.workflow.runs.WorkflowChatContextPayload",
        scopeId: "user-probe",
        conversationId: SERVER_CONVERSATION,
        turnId,
        stateVersion: String(stateVersion),
      },
    },
  };
}

function actionRequestFrame(
  turnId = TURN_ID,
  actionRequestId = `action-${turnId}`,
  serviceSlug = "api-github",
): ChatStreamFrame {
  return {
    type: "CUSTOM",
    custom: {
      name: "nyxid.action.request",
      payload: {
        schemaVersion: 4,
        actorId: "nyxid-chat-workflow-action-probe",
        originTurnId: turnId,
        taskId: `task-${turnId}`,
        stepId: `step-${turnId}`,
        actionRequestId,
        action: "service.connect",
        params: {
          catalogService: {
            serviceSlug,
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
  readonly startCount: number;
  emit(frames: readonly ChatStreamFrame[]): void;
  finish(): void;
}

function installManualStream(): ManualStream {
  let request: ChatStreamRequest | null = null;
  let settle: ((result: { readonly kind: "complete" }) => void) | undefined;
  let startCount = 0;
  vi.spyOn(chatStreamClient, "start").mockImplementation((nextRequest) => {
    startCount += 1;
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
    get startCount() {
      return startCount;
    },
    emit(frames) {
      if (!request) throw new Error("The workflow stream has not started.");
      request.onFrames(frames);
    },
    finish() {
      if (!settle) throw new Error("The workflow stream has not started.");
      const finish = settle;
      settle = undefined;
      finish({ kind: "complete" });
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
  readonly send: (content: string) => Promise<void>;
  readonly setHistoryMode: (mode: HistoryMode) => void;
  readonly setServerMessages: (
    messages: readonly ServerHistoryMessage[],
  ) => void;
  readonly setServerAssistantText: (text: string) => void;
  readonly historyIsMaterialized: () => boolean;
  readonly projectionCanMaterialize: () => boolean;
  readonly materializeProjection: () => Promise<ConversationHistory>;
  readonly unmount: () => void;
}

async function startProbe(): Promise<ProbeSession> {
  useAuthStore.getState().setUser({ id: "user-probe" } as User);
  let historyMode: HistoryMode = "missing";
  let serverMessages = serverTurnMessages(
    TURN_ID,
    "Connect GitHub",
    "Complete the requested action in NyxID, then continue this conversation.",
    1785297207000,
  );
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
            : jsonResponse(serverHistory(serverMessages)),
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
  const send = async (content: string) => {
    const previousStartCount = stream.startCount;
    await act(async () => {
      await hook.result.current.mutateAsync(content);
    });
    await vi.waitFor(() =>
      expect(stream.startCount).toBe(previousStartCount + 1),
    );
  };

  const materializeProjection = async (): Promise<ConversationHistory> => {
    const reconciliation = transport.reconcileProjection(conversation.id);
    // The reconciler defers a post-terminal first observation through its
    // jittered backoff policy, and its due time is computed against the
    // injected (frozen) clock. Advance past any such deferral so the real
    // timer's first fire observes instead of rescheduling forever. Well
    // inside the 15s materialization grace, so grace-sensitive probes are
    // unaffected.
    now += 2_000;
    const requiredTurnId = observedEvents
      .filter((event) => event.event === "turn.completed")
      .at(-1)?.turn_id;
    const canMaterialize = serverMessages.some(
      (message) =>
        message.role === "assistant" && message.turnId === requiredTurnId,
    );
    if (canMaterialize) {
      await reconciliation;
    } else {
      await vi.waitFor(async () => {
        const observed = await transport.getHistory(conversation.id);
        expect(observed.messages[0]?.id).toBe(serverMessages[0]?.id);
      });
      transport.releaseProjectionWaiter(conversation.id);
    }
    const history = await transport.getHistory(conversation.id);
    queryClient.setQueryData(assistantKeys.history(conversation.id), history);
    return history;
  };

  await send("Connect GitHub");

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
    send,
    setHistoryMode: (mode) => {
      historyMode = mode;
    },
    setServerMessages: (messages) => {
      serverMessages = [...messages];
    },
    setServerAssistantText: (text) => {
      serverMessages = serverMessages.map((message) =>
        message.id === `${TURN_ID}:assistant`
          ? { ...message, content: text }
          : message,
      );
    },
    historyIsMaterialized: () => historyMode === "materialized",
    projectionCanMaterialize: () => {
      const requiredTurnId = observedEvents
        .filter((event) => event.event === "turn.completed")
        .at(-1)?.turn_id;
      return serverMessages.some(
        (message) =>
          message.role === "assistant" && message.turnId === requiredTurnId,
      );
    },
    materializeProjection,
    unmount: () => {
      unsubscribe();
      hook.unmount();
      queryClient.clear();
    },
  };
}

async function finishAndWait(
  session: ProbeSession,
  completedTurnCount = 1,
): Promise<void> {
  await act(async () => {
    session.stream.finish();
  });
  await vi.waitFor(() => {
    expect(
      session.observedEvents.filter(
        (event) => event.event === "turn.completed",
      ),
    ).toHaveLength(completedTurnCount);
  });
  await vi.waitFor(() => {
    const episode = session.queryClient.getQueryData<TurnEpisode | null>(
      assistantKeys.episode(session.placeholderId),
    );
    expect(episode?.projecting).toBe(false);
  });
  if (
    session.historyIsMaterialized() &&
    session.projectionCanMaterialize()
  ) {
    await session.materializeProjection();
  }
}

async function switchRead(session: ProbeSession): Promise<ConversationHistory> {
  session.queryClient.removeQueries({
    queryKey: assistantKeys.history(session.placeholderId),
  });
  const history = session.historyIsMaterialized()
    ? await session.materializeProjection()
    : await session.transport.getHistory(session.placeholderId);
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

function actionActivityMessageIds(events: readonly TurnEvent[]): string[] {
  return events.flatMap((event) =>
    event.event === "block.started" && event.block.type === "action_card"
      ? [event.message_id]
      : [],
  );
}

function actionCardBlockIds(history: ConversationHistory): string[] {
  return history.messages.flatMap((message) =>
    message.blocks.flatMap((block) =>
      block.type === "action_card" ? [block.block_id] : [],
    ),
  );
}

function onlyActionActivityMessageId(events: readonly TurnEvent[]): string {
  const ids = actionActivityMessageIds(events);
  if (ids.length !== 1 || !ids[0]) {
    throw new Error(
      `Expected one action activity message, received ${ids.length}.`,
    );
  }
  return ids[0];
}

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  useAuthStore.getState().setUser(null);
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
    ]);
    session.setHistoryMode("materialized");
    session.stream.emit([
      { runFinished: { threadId: RUN_ACTOR, status: "completed" } },
    ]);
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
    const activityMessageId = onlyActionActivityMessageId(
      session.observedEvents,
    );
    const withinGrace = await switchRead(session);
    expect(hasActionCard(withinGrace)).toBe(true);
    expect(withinGrace.messages.map((message) => message.id)).not.toContain(
      `${TURN_ID}:assistant`,
    );
    expect(withinGrace.messages.at(-1)?.id).toBe(activityMessageId);
    session.unmount();
  });

  it("S2 preserves a card-only blocked turn while adopting materialized text in turn order", async () => {
    const session = await startProbe();
    session.stream.emit([
      workflowContextFrame(),
      { runStarted: { threadId: RUN_ACTOR, runId: RUN_ACTOR } },
      actionRequestFrame(),
    ]);
    session.setHistoryMode("materialized");
    session.stream.emit([
      { runFinished: { threadId: RUN_ACTOR, status: "blocked" } },
    ]);
    await finishAndWait(session);

    const activityMessageId = onlyActionActivityMessageId(
      session.observedEvents,
    );
    const expectedOrder = [
      `${TURN_ID}:user`,
      `${TURN_ID}:assistant`,
      activityMessageId,
    ];
    expect(mirrorMintedCard(session.observedEvents)).toBe(true);
    expect(session.cacheSawCardWhileRunning()).toBe(false);
    const projected = session.queryClient.getQueryData<ConversationHistory>(
      assistantKeys.history(session.placeholderId),
    );
    expect(hasActionCard(projected)).toBe(true);
    expect(projected?.messages.map((message) => message.id)).toEqual(
      expectedOrder,
    );
    const withinGrace = await switchRead(session);
    expect(hasActionCard(withinGrace)).toBe(true);
    expect(withinGrace.messages.map((message) => message.id)).toEqual(
      expectedOrder,
    );

    session.advanceHistoryClock(15_001);
    const materialized = await switchRead(session);
    expect(hasActionCard(materialized)).toBe(true);
    expect(materialized.messages.map((message) => message.id)).toEqual(
      expectedOrder,
    );
    expect(
      materialized.messages.find(
        (message) => message.id === `${TURN_ID}:assistant`,
      ),
    ).toMatchObject({ turnId: TURN_ID, status: "blocked" });

    session.setServerAssistantText("The server transcript caught up.");
    const converged = await switchRead(session);
    expect(hasActionCard(converged)).toBe(true);
    expect(converged.messages.map((message) => message.id)).toEqual(
      expectedOrder,
    );
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

  it("anchors two card-only blocked turns after their own server rows", async () => {
    const firstTurnMessages = serverTurnMessages(
      TURN_ID,
      "Connect GitHub",
      "GitHub needs your confirmation.",
      1785297207000,
    );
    const secondTurnMessages = serverTurnMessages(
      SECOND_TURN_ID,
      "Connect Slack",
      "Slack needs your confirmation.",
      1785297210000,
    );
    const session = await startProbe();
    session.stream.emit([
      workflowContextFrame(),
      { runStarted: { threadId: RUN_ACTOR, runId: RUN_ACTOR } },
      actionRequestFrame(),
    ]);
    session.setServerMessages(firstTurnMessages);
    session.setHistoryMode("materialized");
    session.stream.emit([
      { runFinished: { threadId: RUN_ACTOR, status: "blocked" } },
    ]);
    await finishAndWait(session);

    const firstActivityMessageId = onlyActionActivityMessageId(
      session.observedEvents,
    );
    expect(
      (await switchRead(session)).messages.map((message) => message.id),
    ).toEqual([
      `${TURN_ID}:user`,
      `${TURN_ID}:assistant`,
      firstActivityMessageId,
    ]);

    await session.send("Connect Slack");
    session.stream.emit([
      workflowContextFrame(SECOND_TURN_ID, 5),
      { runStarted: { threadId: RUN_ACTOR, runId: RUN_ACTOR } },
      actionRequestFrame(
        SECOND_TURN_ID,
        `action-${SECOND_TURN_ID}`,
        "api-slack",
      ),
    ]);
    session.setServerMessages([...firstTurnMessages, ...secondTurnMessages]);
    session.stream.emit([
      { runFinished: { threadId: RUN_ACTOR, status: "blocked" } },
    ]);
    await finishAndWait(session, 2);

    const activityMessageIds = actionActivityMessageIds(session.observedEvents);
    expect(activityMessageIds).toHaveLength(2);
    const secondActivityMessageId = activityMessageIds[1];
    if (!secondActivityMessageId) {
      throw new Error("Expected a second action activity message.");
    }
    const expectedOrder = [
      `${TURN_ID}:user`,
      `${TURN_ID}:assistant`,
      firstActivityMessageId,
      `${SECOND_TURN_ID}:user`,
      `${SECOND_TURN_ID}:assistant`,
      secondActivityMessageId,
    ];
    const withinGrace = await switchRead(session);
    expect(withinGrace.messages.map((message) => message.id)).toEqual(
      expectedOrder,
    );
    expect(withinGrace.messages.map((message) => message.role)).toEqual([
      "user",
      "assistant",
      "assistant",
      "user",
      "assistant",
      "assistant",
    ]);

    session.advanceHistoryClock(15_001);
    const postGrace = await switchRead(session);
    expect(postGrace.messages.map((message) => message.id)).toEqual(
      expectedOrder,
    );
    expect(actionCardBlockIds(postGrace)).toHaveLength(2);
    session.unmount();
  });

  it("appends an activity for an unmaterialized turn, then moves it to its turn anchor", async () => {
    const priorTurnId = "turn-prior-materialized";
    const priorTurnMessages = serverTurnMessages(
      priorTurnId,
      "Earlier question",
      "Earlier answer",
      1785297200000,
    );
    const currentTurnMessages = serverTurnMessages(
      TURN_ID,
      "Connect GitHub",
      "GitHub needs your confirmation.",
      1785297207000,
    );
    const session = await startProbe();
    session.stream.emit([
      workflowContextFrame(),
      { runStarted: { threadId: RUN_ACTOR, runId: RUN_ACTOR } },
      actionRequestFrame(),
    ]);
    session.setServerMessages(priorTurnMessages);
    session.setHistoryMode("materialized");
    session.stream.emit([
      { runFinished: { threadId: RUN_ACTOR, status: "blocked" } },
    ]);
    await finishAndWait(session);

    const activityMessageId = onlyActionActivityMessageId(
      session.observedEvents,
    );
    const firstRead = await switchRead(session);
    const [actionBlockId] = actionCardBlockIds(firstRead);
    if (!actionBlockId) throw new Error("Expected an action-card block.");
    expect(firstRead.messages.map((message) => message.id)).toEqual([
      `${priorTurnId}:user`,
      `${priorTurnId}:assistant`,
      activityMessageId,
    ]);

    session.setServerMessages([...priorTurnMessages, ...currentTurnMessages]);
    const materialized = await switchRead(session);
    expect(materialized.messages.map((message) => message.id)).toEqual([
      `${priorTurnId}:user`,
      `${priorTurnId}:assistant`,
      `${TURN_ID}:user`,
      `${TURN_ID}:assistant`,
      activityMessageId,
    ]);
    expect(actionCardBlockIds(materialized)).toEqual([actionBlockId]);
    expect(
      materialized.messages.filter(
        (message) => message.id === activityMessageId,
      ),
    ).toHaveLength(1);
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

describe("empty-stream late materialization", () => {
  it("renders a late-materializing answer into the open thread with no refresh", async () => {
    // The production trace this pins: POST /workflow-chat answers 200, the
    // stream adopts the durable chatc- id and then closes without a single
    // printable frame — yet the reply exists upstream and the transcript
    // materializes moments later. The mounted conversation must receive that
    // answer through the reconciler's own invalidation: no reload, no manual
    // refetch, no navigation. The canonical-id swap mid-flight (release the
    // placeholder waiter, re-acquire under the canonical key in the same
    // commit) is included deliberately — it is the interleaving that used to
    // orphan the reconciler and force the refresh.
    useAuthStore.getState().setUser({ id: "user-probe" } as User);
    const ANSWER = "Here is the answer that materialized late.";

    type HistoryFetchMode = "missing" | "hanging" | "materialized";
    let historyFetchMode: HistoryFetchMode = "missing";
    let hangingStarted = false;
    const serverMessages = serverTurnMessages(
      TURN_ID,
      "hi",
      ANSWER,
      1785297207000,
    );
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
          if (historyFetchMode === "hanging") {
            hangingStarted = true;
            // Pends until aborted, like a slow real request: the abort must
            // reject asynchronously so the release/re-acquire race is real.
            return new Promise<Response>((_resolve, reject) => {
              const abort = () => {
                reject(new DOMException("aborted", "AbortError"));
              };
              if (init?.signal?.aborted) {
                abort();
                return;
              }
              init?.signal?.addEventListener("abort", abort);
            });
          }
          return Promise.resolve(
            historyFetchMode === "missing"
              ? jsonResponse(
                  { error: "not_found", error_code: -1, message: "404" },
                  404,
                )
              : jsonResponse(serverHistory(serverMessages)),
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

    const transport = new AevatarAssistantTransport();
    const conversation = await transport.createConversation();
    const observedEvents: TurnEvent[] = [];
    bindHookTransport(transport, observedEvents);
    vi.spyOn(assistantTransport, "reconcileProjection").mockImplementation(
      (id) => transport.reconcileProjection(id),
    );
    vi.spyOn(assistantTransport, "releaseProjectionWaiter").mockImplementation(
      (id) => {
        transport.releaseProjectionWaiter(id);
      },
    );
    const stream = installManualStream();
    const { queryClient, Wrapper } = createHarness();

    const hook = renderHook(
      ({ conversationId }: { readonly conversationId: string }) => ({
        send: useSendMessage(conversationId),
        history: useConversation(conversationId),
      }),
      {
        wrapper: Wrapper,
        initialProps: { conversationId: conversation.id },
      },
    );

    await act(async () => {
      await hook.result.current.send.mutateAsync("hi");
    });
    await vi.waitFor(() => {
      expect(stream.startCount).toBe(1);
    });

    // Durable id adopted, then the stream dies with zero printable frames.
    await act(async () => {
      stream.emit([workflowContextFrame()]);
    });
    historyFetchMode = "hanging";
    await act(async () => {
      stream.finish();
    });
    await vi.waitFor(() => {
      expect(
        observedEvents.filter((event) => event.event === "turn.completed"),
      ).toHaveLength(1);
    });

    // The post-terminal projection marks the mirror awaiting projection and
    // the mounted hook starts the reconciler; wait for its transcript GET to
    // be genuinely in flight before swapping keys.
    await vi.waitFor(
      () => {
        expect(hangingStarted).toBe(true);
      },
      { timeout: 5_000 },
    );

    // The page's canonical-id swap: copy the cache under the canonical key
    // (assistant.tsx does this before navigating), then remount the hooks on
    // the canonical id. Cleanup releases the placeholder waiter — aborting
    // the in-flight GET — and the new effect re-acquires in the same commit.
    const placeholderData = queryClient.getQueryData<ConversationHistory>(
      assistantKeys.history(conversation.id),
    );
    expect(placeholderData?.awaitingProjection).toBe(true);
    queryClient.setQueryData(
      assistantKeys.history(SERVER_CONVERSATION),
      placeholderData,
    );
    historyFetchMode = "materialized";
    hook.rerender({ conversationId: SERVER_CONVERSATION });

    // No cache writes and no manual refetch past this point: the reconciler
    // must survive the swap, materialize the transcript, and its invalidation
    // must repopulate the mounted query on its own.
    await vi.waitFor(
      () => {
        const messages = hook.result.current.history.data?.messages ?? [];
        expect(
          messages.some(
            (message) =>
              message.role === "assistant" &&
              message.blocks.some(
                (block) => block.type === "text" && block.text === ANSWER,
              ),
          ),
        ).toBe(true);
      },
      { timeout: 10_000 },
    );

    hook.unmount();
    queryClient.clear();
  });
});
