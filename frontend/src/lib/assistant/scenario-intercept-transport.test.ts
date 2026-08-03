import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AevatarAssistantTransport } from "@/lib/assistant/aevatar-transport";
import {
  AssistantConversationNotFoundError,
  AssistantTurnActiveError,
} from "@/lib/assistant/errors";
import { compileScenarioSet, scenario } from "@/lib/assistant/scenario-engine";
import {
  MockScenariosLoadingError,
  ScenarioInterceptTransport,
} from "@/lib/assistant/scenario-intercept-transport";
import { useAssistantMockScenariosStore } from "@/stores/assistant-mock-scenarios-store";
import type { ActionReport } from "@/schemas/assistant-actions";
import type {
  ActionCardContentBlock,
  ApprovalCardContentBlock,
  AssistantMessage,
  AssistantTransport,
  Conversation,
  ConversationHistory,
  TurnEvent,
  TurnHandle,
} from "@/types/assistant";

const CONVERSATION_ID = "chatc-real-1";
const CREATED_AT = "2026-08-04T00:00:00.000Z";

function textMessage(
  id: string,
  role: AssistantMessage["role"],
  text: string,
  createdAt = CREATED_AT,
): AssistantMessage {
  return {
    id,
    role,
    schema_version: 1,
    blocks: [{ type: "text", block_id: `${id}-block`, text }],
    created_at: createdAt,
  };
}

function baseConversation(
  id = CONVERSATION_ID,
  overrides: Partial<Conversation> = {},
): Conversation {
  return {
    id,
    title: "Existing conversation",
    created_at: CREATED_AT,
    last_message_at: "2026-08-04T00:01:00.000Z",
    message_count: 2,
    ...overrides,
  };
}

function baseHistory(id = CONVERSATION_ID): ConversationHistory {
  return {
    conversation: baseConversation(id),
    messages: [
      textMessage("real-user-1", "user", "Earlier question"),
      textMessage("real-assistant-1", "assistant", "Earlier answer"),
    ],
    has_more: false,
  };
}

class StubTransport implements AssistantTransport {
  histories = new Map<string, ConversationHistory>([
    [CONVERSATION_ID, baseHistory()],
  ]);
  conversations: Conversation[] = [baseConversation()];
  autoCompleteSends = true;
  appendLiveMessages = false;
  deleteImpl: (conversationId: string) => Promise<void> = async () => undefined;
  historyCalls = 0;
  listCalls = 0;
  createCalls = 0;
  deleteCalls = 0;
  sendCalls = 0;
  cancelCalls = 0;
  approvalCalls = 0;
  progressCalls = 0;
  blockCalls = 0;
  continueCalls = 0;
  wakeCalls = 0;
  lastDelegateEvent: ((event: TurnEvent) => void) | null = null;
  private cursor = 0;

  async listConversations(): Promise<Conversation[]> {
    this.listCalls += 1;
    return structuredClone(this.conversations);
  }

  async createConversation(): Promise<Conversation> {
    this.createCalls += 1;
    const id = `workflow-pending-${String(this.createCalls)}`;
    const conversation = baseConversation(id, {
      title: "New chat",
      message_count: undefined,
    });
    this.conversations = [conversation, ...this.conversations];
    this.histories.set(id, {
      conversation,
      messages: [],
      has_more: false,
    });
    return structuredClone(conversation);
  }

  async getHistory(conversationId: string): Promise<ConversationHistory> {
    this.historyCalls += 1;
    const history = this.histories.get(conversationId);
    if (!history) throw new AssistantConversationNotFoundError();
    return structuredClone(history);
  }

  async deleteConversation(conversationId: string): Promise<void> {
    this.deleteCalls += 1;
    await this.deleteImpl(conversationId);
    this.histories.delete(conversationId);
    this.conversations = this.conversations.filter(
      (conversation) => conversation.id !== conversationId,
    );
  }

  sendMessage(
    conversationId: string,
    content: string,
    onEvent: (event: TurnEvent) => void,
  ): TurnHandle {
    this.sendCalls += 1;
    this.lastDelegateEvent = onEvent;
    if (this.appendLiveMessages) {
      const history = this.histories.get(conversationId);
      if (history) {
        const suffix = String(this.sendCalls);
        const messages = [
          ...history.messages,
          textMessage(
            `live-user-${suffix}`,
            "user",
            content,
            `2026-08-04T00:0${String(2 + this.sendCalls)}:00.000Z`,
          ),
          textMessage(
            `live-assistant-${suffix}`,
            "assistant",
            `Live answer ${suffix}`,
            `2026-08-04T00:0${String(3 + this.sendCalls)}:00.000Z`,
          ),
        ];
        const conversation = {
          ...history.conversation,
          last_message_at: messages.at(-1)?.created_at ?? CREATED_AT,
          message_count: messages.length,
        };
        this.histories.set(conversationId, {
          ...history,
          conversation,
          messages,
        });
        this.conversations = this.conversations.map((entry) =>
          entry.id === conversationId ? conversation : entry,
        );
      }
    }
    this.cursor += 1;
    const turnId = `delegate-turn-${String(this.sendCalls)}`;
    onEvent({
      cursor: this.cursor,
      event: "turn.status",
      turn_id: turnId,
      status: "running",
    });
    if (this.autoCompleteSends) this.completeDelegateTurn();
    return { turnId, cancel: () => this.cancelActiveTurn() };
  }

  completeDelegateTurn(): void {
    if (!this.lastDelegateEvent) return;
    this.cursor += 1;
    this.lastDelegateEvent({
      cursor: this.cursor,
      event: "turn.completed",
      turn_id: `delegate-turn-${String(this.sendCalls)}`,
      status: "completed",
      error: null,
    });
  }

  cancelActiveTurn(): void {
    this.cancelCalls += 1;
  }

  async decideApproval(): Promise<TurnHandle | null> {
    this.approvalCalls += 1;
    return null;
  }

  setActionCardInProgress(): void {
    this.progressCalls += 1;
  }

  blockActionCard(): void {
    this.blockCalls += 1;
  }

  continueActions(): TurnHandle | null {
    this.continueCalls += 1;
    return null;
  }

  wakeActions(): TurnHandle {
    this.wakeCalls += 1;
    return { turnId: "delegate-wake", cancel: () => undefined };
  }
}

function resetScenarioStore(): void {
  useAssistantMockScenariosStore.setState({
    enabled: true,
    disabledScenarioIds: [],
    world: { connected: [] },
    userId: "user-a",
    engineState: "ready",
    lastActivity: null,
  });
}

async function createTransport(
  delegate = new StubTransport(),
  options: ConstructorParameters<typeof ScenarioInterceptTransport>[1] = {
    verifyKey: async () => ({ catalog_service_slug: "api-github" }),
  },
): Promise<{
  readonly delegate: StubTransport;
  readonly transport: ScenarioInterceptTransport;
}> {
  const transport = new ScenarioInterceptTransport(delegate, options);
  await transport.getHistory(CONVERSATION_ID);
  await transport.listConversations();
  return { delegate, transport };
}

function findActionCard(events: readonly TurnEvent[]): ActionCardContentBlock {
  const event = events.find(
    (candidate) =>
      candidate.event === "block.completed" &&
      candidate.block.type === "action_card",
  );
  if (
    event?.event !== "block.completed" ||
    event.block.type !== "action_card"
  ) {
    throw new Error("Expected an action card.");
  }
  return event.block;
}

function findApprovalCard(
  events: readonly TurnEvent[],
): ApprovalCardContentBlock {
  const event = events.find(
    (candidate) =>
      candidate.event === "block.completed" &&
      candidate.block.type === "approval_card",
  );
  if (
    event?.event !== "block.completed" ||
    event.block.type !== "approval_card"
  ) {
    throw new Error("Expected an approval card.");
  }
  return event.block;
}

function report(
  card: ActionCardContentBlock,
  disposition: ActionReport["disposition"],
): ActionReport {
  return {
    actionRequestId: card.action_request_id,
    originTurnId: card.origin_turn_id,
    disposition,
    ...(disposition === "completed"
      ? { resource: { userService: { userServiceId: "service-1" } } }
      : {}),
  };
}

async function parkGitHubAction(
  transport: ScenarioInterceptTransport,
  conversationId = CONVERSATION_ID,
): Promise<{
  readonly card: ActionCardContentBlock;
  readonly events: TurnEvent[];
}> {
  const events: TurnEvent[] = [];
  transport.sendMessage(conversationId, "connect to my github", (event) =>
    events.push(event),
  );
  await vi.runAllTimersAsync();
  return { card: findActionCard(events), events };
}

async function flushAsync(): Promise<void> {
  await Promise.resolve();
  await Promise.resolve();
}

describe("ScenarioInterceptTransport ownership", () => {
  it("passes unmatched messages through and tracks delegate completion", async () => {
    const { delegate, transport } = await createTransport();

    transport.sendMessage(CONVERSATION_ID, "unmatched live request", () => {});
    transport.sendMessage(CONVERSATION_ID, "another live request", () => {});

    expect(delegate.sendCalls).toBe(2);
    expect(
      useAssistantMockScenariosStore.getState().lastActivity,
    ).toMatchObject({
      matched: false,
      scenarioId: null,
    });
  });

  it("rejects send while a mock turn is running (§6.2)", async () => {
    const { delegate, transport } = await createTransport();

    transport.sendMessage(CONVERSATION_ID, "simulate an error", () => {});

    expect(() =>
      transport.sendMessage(CONVERSATION_ID, "another message", () => {}),
    ).toThrow(AssistantTurnActiveError);
    expect(delegate.sendCalls).toBe(0);
  });

  it("rejects resume while a mock continuation is running (§6.2)", async () => {
    const { delegate, transport } = await createTransport();
    const { card } = await parkGitHubAction(transport);
    const failedReport = report(card, "failed");
    transport.continueActions(CONVERSATION_ID, card.origin_turn_id, [
      failedReport,
    ]);

    expect(() =>
      transport.continueActions(CONVERSATION_ID, card.origin_turn_id, [
        failedReport,
      ]),
    ).toThrow(AssistantTurnActiveError);
    expect(delegate.continueCalls).toBe(0);
  });

  it("routes mock wakeActions and keeps cursors monotonic across resume (F1)", async () => {
    const delegate = new StubTransport();
    const compiled = compileScenarioSet(
      [
        scenario("wake", /wake/, (script) =>
          script
            .action("service.connect", { service: "api-github" })
            .await({ wake: (branch) => branch.say("Explicit wake branch") })
            .say("After wake"),
        ),
      ],
      {},
    );
    const { transport } = await createTransport(delegate, { compiled });
    const events: TurnEvent[] = [];
    transport.sendMessage(CONVERSATION_ID, "wake", (event) =>
      events.push(event),
    );
    await vi.runAllTimersAsync();
    const card = findActionCard(events);
    const parkedEventCount = events.length;
    const parkedCursor = events.at(-1)?.cursor ?? 0;

    const handle = transport.wakeActions(
      CONVERSATION_ID,
      card.origin_turn_id,
      (event) => events.push(event),
    );
    await vi.runAllTimersAsync();

    expect(handle.turnId).not.toBe(card.origin_turn_id);
    expect(delegate.wakeCalls).toBe(0);
    expect(events[parkedEventCount]?.cursor).toBeGreaterThan(parkedCursor);
    expect(
      events.every(
        (event, index) => index === 0 || event.cursor > events[index - 1]!.cursor,
      ),
    ).toBe(true);
    expect(events).toContainEqual(
      expect.objectContaining({
        event: "block.completed",
        block: expect.objectContaining({
          type: "text",
          text: "Explicit wake branch",
        }),
      }),
    );
  });

  it("routes Stop, card patches, approval decisions, and action reports after toggle-off (F2)", async () => {
    const { delegate, transport } = await createTransport();
    const runningEvents: TurnEvent[] = [];
    transport.sendMessage(CONVERSATION_ID, "simulate an error", (event) =>
      runningEvents.push(event),
    );
    useAssistantMockScenariosStore.getState().setEnabled(false);
    transport.cancelActiveTurn(CONVERSATION_ID);
    expect(runningEvents.at(-1)).toMatchObject({
      event: "turn.completed",
      status: "cancelled",
    });

    useAssistantMockScenariosStore.getState().setEnabled(true);
    const action = await parkGitHubAction(transport);
    useAssistantMockScenariosStore.getState().setEnabled(false);
    transport.setActionCardInProgress(
      CONVERSATION_ID,
      action.card.block_id,
      true,
    );
    transport.blockActionCard(
      CONVERSATION_ID,
      action.card.block_id,
      "Dialog closed",
    );
    transport.continueActions(CONVERSATION_ID, action.card.origin_turn_id, [
      report(action.card, "failed"),
    ]);
    await vi.runAllTimersAsync();

    useAssistantMockScenariosStore.getState().setEnabled(true);
    const completedAction = await parkGitHubAction(transport);
    useAssistantMockScenariosStore.getState().setEnabled(false);
    transport.continueActions(
      CONVERSATION_ID,
      completedAction.card.origin_turn_id,
      [report(completedAction.card, "completed")],
    );
    await flushAsync();
    await vi.runAllTimersAsync();

    for (const approved of [true, false]) {
      useAssistantMockScenariosStore.getState().setEnabled(true);
      const approvalEvents: TurnEvent[] = [];
      transport.sendMessage(CONVERSATION_ID, "post weekly digest", (event) =>
        approvalEvents.push(event),
      );
      await vi.runAllTimersAsync();
      const approval = findApprovalCard(approvalEvents);
      useAssistantMockScenariosStore.getState().setEnabled(false);
      await transport.decideApproval(
        CONVERSATION_ID,
        approval.block_id,
        approved,
      );
      await vi.runAllTimersAsync();
    }

    expect(delegate).toMatchObject({
      cancelCalls: 0,
      progressCalls: 0,
      blockCalls: 0,
      continueCalls: 0,
      approvalCalls: 0,
    });
  });

  it("settles a parked card before a newer delegated send and rejects mock resume during delegate activity (F3)", async () => {
    const delegate = new StubTransport();
    const { transport } = await createTransport(delegate);
    const { card, events } = await parkGitHubAction(transport);
    useAssistantMockScenariosStore.getState().setEnabled(false);
    delegate.autoCompleteSends = false;

    transport.sendMessage(CONVERSATION_ID, "go live now", () => {});

    expect(events.at(-1)).toMatchObject({
      event: "block.updated",
      patch: {
        status: "conflicted",
        outcome_note: "Superseded by a newer message.",
      },
    });
    expect(() =>
      transport.continueActions(CONVERSATION_ID, card.origin_turn_id, [
        report(card, "failed"),
      ]),
    ).toThrow(AssistantTurnActiveError);
    expect(delegate.continueCalls).toBe(0);
  });

  it("claims a mock-first placeholder and never delegates later sends (F4)", async () => {
    const delegate = new StubTransport();
    const transport = new ScenarioInterceptTransport(delegate, {
      verifyKey: async () => ({ catalog_service_slug: "api-github" }),
    });
    const conversation = await transport.createConversation();
    const events: TurnEvent[] = [];

    transport.sendMessage(conversation.id, "simulate error", (event) =>
      events.push(event),
    );
    await vi.runAllTimersAsync();
    useAssistantMockScenariosStore.getState().setEnabled(false);
    transport.sendMessage(conversation.id, "unmatched after claim", (event) =>
      events.push(event),
    );
    await vi.runAllTimersAsync();

    const history = await transport.getHistory(conversation.id);
    expect(delegate.sendCalls).toBe(0);
    expect(
      history.messages.some(
        (message) =>
          message.blocks[0]?.type === "text" &&
          message.blocks[0].text.includes("mock-only chat"),
      ),
    ).toBe(true);
    expect((await transport.listConversations())[0]).toMatchObject({
      id: conversation.id,
      title: "simulate error",
    });

    await transport.deleteConversation(conversation.id);
    await expect(transport.getHistory(conversation.id)).rejects.toBeInstanceOf(
      AssistantConversationNotFoundError,
    );
    const reloaded = new ScenarioInterceptTransport(delegate);
    await expect(reloaded.getHistory(conversation.id)).rejects.toBeInstanceOf(
      AssistantConversationNotFoundError,
    );
  });

  it("throws without delegating while enabled config is loading or failed (F6)", async () => {
    const { delegate, transport } = await createTransport();
    for (const state of ["loading", "error"] as const) {
      useAssistantMockScenariosStore.getState().setEngineState(state);
      expect(() =>
        transport.sendMessage(CONVERSATION_ID, "connect my github", () => {}),
      ).toThrow(MockScenariosLoadingError);
    }
    expect(delegate.sendCalls).toBe(0);
  });
});

describe("ScenarioInterceptTransport projections", () => {
  it("anchors repeated mock/live alternation in chronological order (F7)", async () => {
    const delegate = new StubTransport();
    delegate.appendLiveMessages = true;
    const { transport } = await createTransport(delegate);

    transport.sendMessage(CONVERSATION_ID, "simulate error", () => {});
    await vi.runAllTimersAsync();
    await transport.getHistory(CONVERSATION_ID);
    transport.sendMessage(CONVERSATION_ID, "live middle", () => {});
    await transport.getHistory(CONVERSATION_ID);
    transport.sendMessage(CONVERSATION_ID, "simulate an error", () => {});
    await vi.runAllTimersAsync();

    const history = await transport.getHistory(CONVERSATION_ID);
    const texts = history.messages.map((message) =>
      message.blocks[0]?.type === "text"
        ? message.blocks[0].text
        : "structured",
    );
    expect(texts).toEqual([
      "Earlier question",
      "Earlier answer",
      "simulate error",
      "Starting the scripted error rehearsal.",
      "live middle",
      "Live answer 1",
      "simulate an error",
      "Starting the scripted error rehearsal.",
    ]);
  });

  it("serves history snapshots for every scripted event without delegate reads (F8)", async () => {
    const compiled = compileScenarioSet(
      [scenario("long", /long/, (script) => script.say("x".repeat(720)))],
      {},
    );
    const { delegate, transport } = await createTransport(new StubTransport(), {
      compiled,
      verifyKey: async () => ({ catalog_service_slug: "api-github" }),
    });
    const initialHistoryCalls = delegate.historyCalls;
    const projections: Promise<ConversationHistory>[] = [];

    transport.sendMessage(CONVERSATION_ID, "long", () => {
      projections.push(transport.getHistory(CONVERSATION_ID));
    });
    await vi.runAllTimersAsync();
    await Promise.all(projections);

    expect(projections.length).toBeGreaterThan(20);
    expect(delegate.historyCalls).toBe(initialHistoryCalls);
  });

  it("refreshes delegated history and list while preserving a parked overlay", async () => {
    const delegate = new StubTransport();
    const transport = new ScenarioInterceptTransport(delegate);
    await parkGitHubAction(transport);
    const refreshedConversation = baseConversation(CONVERSATION_ID, {
      title: "Refreshed conversation",
      last_message_at: "2026-08-04T00:30:00.000Z",
      message_count: 7,
    });
    delegate.histories.set(CONVERSATION_ID, {
      ...baseHistory(),
      conversation: refreshedConversation,
    });
    delegate.conversations = [refreshedConversation];

    const history = await transport.getHistory(CONVERSATION_ID);
    const listed = (await transport.listConversations()).find(
      (conversation) => conversation.id === CONVERSATION_ID,
    );

    expect(delegate.historyCalls).toBe(1);
    expect(delegate.listCalls).toBe(1);
    expect(history.conversation.title).toBe("Refreshed conversation");
    expect(history.messages).toContainEqual(
      expect.objectContaining({
        role: "user",
        blocks: expect.arrayContaining([
          expect.objectContaining({
            type: "text",
            text: "connect to my github",
          }),
        ]),
      }),
    );
    expect(listed).toMatchObject({
      title: "Refreshed conversation",
      message_count: history.conversation.message_count,
    });
    expect(listed?.message_count).toBeGreaterThan(7);
  });

  it("projects claimed title, recency, ordering, and matching history/list counts (F9, P12)", async () => {
    const delegate = new StubTransport();
    const transport = new ScenarioInterceptTransport(delegate, {
      now: () => Date.parse("2026-08-04T02:00:00.000Z"),
    });
    await transport.getHistory(CONVERSATION_ID);
    await transport.listConversations();
    useAssistantMockScenariosStore.getState().connectService("api-github");
    transport.sendMessage(CONVERSATION_ID, "show github issues", () => {});
    await vi.runAllTimersAsync();

    const history = await transport.getHistory(CONVERSATION_ID);
    const listed = (await transport.listConversations()).find(
      (conversation) => conversation.id === CONVERSATION_ID,
    );
    expect(history.conversation).toMatchObject({
      last_message_at: "2026-08-04T02:00:00.000Z",
      message_count: 4,
    });
    expect(listed).toMatchObject({
      last_message_at: "2026-08-04T02:00:00.000Z",
      message_count: 4,
    });

    const claimed = await transport.createConversation();
    transport.sendMessage(
      claimed.id,
      "simulate error with a useful title",
      () => {},
    );
    await vi.runAllTimersAsync();
    const claimedHistory = await transport.getHistory(claimed.id);
    const claimedList = (await transport.listConversations()).find(
      (conversation) => conversation.id === claimed.id,
    );
    expect(claimedHistory.conversation).toMatchObject({
      title: "simulate error with a useful title",
      message_count: 2,
    });
    expect(claimedList?.message_count).toBe(2);
    expect((await transport.listConversations())[0]?.id).toBe(claimed.id);
  });

  it("holds a real Aevatar list snapshot beyond its 5s TTL during mock activity (P7)", async () => {
    vi.setSystemTime(new Date("2026-08-04T03:00:00.000Z"));
    const listFetches: string[] = [];
    vi.stubGlobal(
      "fetch",
      vi.fn((input: RequestInfo | URL) => {
        const url = String(input);
        if (url === "/api/v1/assistant/conversations") {
          listFetches.push(url);
          return Promise.resolve(
            new Response(
              JSON.stringify({
                conversations: [
                  {
                    id: CONVERSATION_ID,
                    title: "Existing conversation",
                    createdAt: CREATED_AT,
                    updatedAt: "2026-08-04T00:01:00.000Z",
                    messageCount: 2,
                  },
                ],
              }),
              { status: 200, headers: { "Content-Type": "application/json" } },
            ),
          );
        }
        return Promise.resolve(new Response("{}", { status: 200 }));
      }),
    );
    const compiled = compileScenarioSet(
      [
        scenario("slow", /slow/, (script) =>
          script.say("start").wait(6_000).say("end"),
        ),
      ],
      {},
    );
    const transport = new ScenarioInterceptTransport(
      new AevatarAssistantTransport(),
      { compiled },
    );
    await transport.listConversations();

    transport.sendMessage(CONVERSATION_ID, "slow", () => {
      void transport.listConversations();
    });
    await vi.advanceTimersByTimeAsync(7_000);

    expect(listFetches).toHaveLength(1);
  });
});

describe("ScenarioInterceptTransport journey verification", () => {
  it.each([
    ["api-github", true, "verified"],
    ["api-openai", false, "different service"],
  ] as const)(
    "uses GET /keys/{id} KeyInfo catalog_service_slug for %s (F5, P1, P2)",
    async (catalogSlug, connected, note) => {
      const fetchMock = vi.fn(() =>
        Promise.resolve(
          new Response(JSON.stringify({ catalog_service_slug: catalogSlug }), {
            status: 200,
            headers: { "Content-Type": "application/json" },
          }),
        ),
      );
      vi.stubGlobal("fetch", fetchMock);
      const { transport } = await createTransport(new StubTransport(), {});
      const { card, events } = await parkGitHubAction(transport);

      const handle = transport.continueActions(
        CONVERSATION_ID,
        card.origin_turn_id,
        [report(card, "completed")],
        (event) => events.push(event),
      );

      expect(handle).not.toBeNull();
      expect(handle?.turnId).toBeNull();
      expect(events.at(-1)).toMatchObject({
        event: "block.updated",
        patch: {
          status: "completed",
          outcome_note: "Reported — awaiting assistant verification.",
        },
      });
      await flushAsync();
      await vi.runAllTimersAsync();

      expect(fetchMock).toHaveBeenCalledWith(
        "/api/v1/keys/service-1",
        expect.objectContaining({
          method: "GET",
          signal: expect.any(AbortSignal),
        }),
      );
      expect(
        useAssistantMockScenariosStore
          .getState()
          .world.connected.includes("api-github"),
      ).toBe(connected);
      if (!connected) {
        expect(
          events.some(
            (event) =>
              event.event === "block.updated" &&
              "outcome_note" in event.patch &&
              String(event.patch.outcome_note).toLowerCase().includes(note),
          ),
        ).toBe(true);
      }
    },
  );

  it.each([
    [
      "404",
      new Response(
        JSON.stringify({ message: "missing", error: "x", error_code: 1 }),
        { status: 404 },
      ),
    ],
    [
      "malformed",
      new Response(JSON.stringify({ slug: "api-github" }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      }),
    ],
  ] as const)(
    "continues as unverified for a %s key lookup response (F5, P1)",
    async (_case, response) => {
      vi.stubGlobal(
        "fetch",
        vi.fn(() => Promise.resolve(response.clone())),
      );
      const { transport } = await createTransport(new StubTransport(), {});
      const { card, events } = await parkGitHubAction(transport);

      transport.continueActions(
        CONVERSATION_ID,
        card.origin_turn_id,
        [report(card, "completed")],
        (event) => events.push(event),
      );
      await flushAsync();
      await vi.runAllTimersAsync();

      expect(useAssistantMockScenariosStore.getState().world.connected).toEqual(
        [],
      );
      expect(events).toContainEqual(
        expect.objectContaining({
          event: "block.updated",
          patch: expect.objectContaining({
            outcome_note:
              "The connection was reported, but could not be verified.",
          }),
        }),
      );
    },
  );

  it("cancels a provisional handle without delivering after the lookup resolves (P2)", async () => {
    let resolveLookup: (value: unknown) => void = () => undefined;
    const lookup = new Promise<unknown>((resolve) => {
      resolveLookup = resolve;
    });
    const { transport } = await createTransport(new StubTransport(), {
      verifyKey: () => lookup,
    });
    const { card, events } = await parkGitHubAction(transport);
    const before = events.length;
    const handle = transport.continueActions(
      CONVERSATION_ID,
      card.origin_turn_id,
      [report(card, "completed")],
      (event) => events.push(event),
    );

    handle?.cancel();
    const afterCancel = events.length;
    resolveLookup({ catalog_service_slug: "api-github" });
    await flushAsync();
    await vi.runAllTimersAsync();

    expect(afterCancel).toBeGreaterThan(before);
    expect(events).toHaveLength(afterCancel);
    expect(useAssistantMockScenariosStore.getState().world.connected).toEqual(
      [],
    );
  });

  it("aborts and suppresses verification when delete wins the lookup race (P2)", async () => {
    const delegate = new StubTransport();
    const verifyKey = vi.fn(
      (_keyId: string, signal: AbortSignal) =>
        new Promise<unknown>((resolve) => {
          signal.addEventListener("abort", () =>
            resolve({ catalog_service_slug: "api-github" }),
          );
        }),
    );
    const { transport } = await createTransport(delegate, { verifyKey });
    const { card } = await parkGitHubAction(transport);
    transport.continueActions(CONVERSATION_ID, card.origin_turn_id, [
      report(card, "completed"),
    ]);

    await transport.deleteConversation(CONVERSATION_ID);
    await flushAsync();
    await vi.runAllTimersAsync();

    expect(verifyKey.mock.calls[0]?.[1].aborted).toBe(true);
    expect(useAssistantMockScenariosStore.getState().world.connected).toEqual(
      [],
    );
    await expect(transport.getHistory(CONVERSATION_ID)).rejects.toBeInstanceOf(
      AssistantConversationNotFoundError,
    );
  });
});

describe("ScenarioInterceptTransport deletion", () => {
  it("leaves unowned state inert after a rejected delegated delete", async () => {
    const delegate = new StubTransport();
    delegate.deleteImpl = async () => {
      throw new Error("delete failed");
    };
    const { transport } = await createTransport(delegate);
    const historyCalls = delegate.historyCalls;
    const listCalls = delegate.listCalls;

    await expect(transport.deleteConversation(CONVERSATION_ID)).rejects.toThrow(
      "delete failed",
    );
    const history = await transport.getHistory(CONVERSATION_ID);
    const conversations = await transport.listConversations();

    expect(delegate.deleteCalls).toBe(1);
    expect(delegate.historyCalls).toBe(historyCalls + 1);
    expect(delegate.listCalls).toBe(listCalls + 1);
    expect(history).toEqual(baseHistory());
    expect(conversations).toContainEqual(baseConversation());
  });

  it("cancels synchronously before a delayed delegate delete (F13)", async () => {
    let resolveDelete: () => void = () => undefined;
    const delegate = new StubTransport();
    delegate.deleteImpl = () =>
      new Promise<void>((resolve) => {
        resolveDelete = resolve;
      });
    const compiled = compileScenarioSet(
      [
        scenario("slow", /slow/, (script) =>
          script.say("start").wait(5_000).say("late"),
        ),
      ],
      {},
    );
    const { transport } = await createTransport(delegate, { compiled });
    const events: TurnEvent[] = [];
    transport.sendMessage(CONVERSATION_ID, "slow", (event) =>
      events.push(event),
    );
    const deletion = transport.deleteConversation(CONVERSATION_ID);
    const afterCancel = events.length;

    await vi.advanceTimersByTimeAsync(6_000);
    expect(events).toHaveLength(afterCancel);
    expect(events.at(-1)).toMatchObject({
      event: "turn.completed",
      status: "cancelled",
    });
    resolveDelete();
    await deletion;
  });

  it("keeps a tombstone after rejected DELETE and prevents resurrection (P10)", async () => {
    const delegate = new StubTransport();
    delegate.deleteImpl = async () => {
      throw new Error("delete failed");
    };
    const { transport } = await createTransport(delegate);
    const { card } = await parkGitHubAction(transport);

    await expect(transport.deleteConversation(CONVERSATION_ID)).rejects.toThrow(
      "delete failed",
    );
    expect(() =>
      transport.sendMessage(CONVERSATION_ID, "unmatched", () => {}),
    ).toThrow(AssistantConversationNotFoundError);
    expect(() =>
      transport.continueActions(CONVERSATION_ID, card.origin_turn_id, [
        report(card, "failed"),
      ]),
    ).toThrow(AssistantConversationNotFoundError);
    transport.setActionCardInProgress(CONVERSATION_ID, card.block_id, true);
    await expect(transport.getHistory(CONVERSATION_ID)).rejects.toBeInstanceOf(
      AssistantConversationNotFoundError,
    );
    expect(
      (await transport.listConversations()).some(
        (conversation) => conversation.id === CONVERSATION_ID,
      ),
    ).toBe(false);
    expect(delegate).toMatchObject({
      sendCalls: 0,
      continueCalls: 0,
      progressCalls: 0,
    });
  });
});

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date("2026-08-04T01:00:00.000Z"));
  vi.unstubAllGlobals();
  localStorage.clear();
  resetScenarioStore();
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
});
